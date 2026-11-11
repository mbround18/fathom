/**
 * Pieces every Repository tab shares (spec 066 T051): links between items, the side detail pane,
 * and the tab's "nothing found" state.
 */
import { XIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { EmptyState, LABEL, LINK, useReveal } from '@companion/ui/kit'
import type { ReactNode } from 'react'

import { goTo, select, type RepoTab } from './address'

/** An item's name in a list: selects it. */
export function ItemButton({ item, children }: { item: string; children: ReactNode }) {
  return (
    <button
      type="button"
      className={`${LINK} min-w-0 text-left font-mono break-all`}
      onClick={() => {
        select(item)
      }}
    >
      {children}
    </button>
  )
}

/** A link to another catalog item, on its own tab. */
export function ItemLink({ tab, item, children }: { tab: RepoTab; item: string; children?: ReactNode }) {
  return (
    <button
      type="button"
      className={`${LINK} font-mono break-all`}
      onClick={() => {
        goTo(tab, item)
      }}
    >
      {children ?? item}
    </button>
  )
}

/** The list beside its detail pane: side by side when there is room, stacked on a phone. */
export function Split({ children }: { children: ReactNode }) {
  return <div className="grid min-w-0 items-start gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(0,26rem)]">{children}</div>
}

/** The side detail pane for the selected item. */
export function DetailPane({ label, title, badges, actions, children }: { label: string; title: string; badges?: ReactNode; actions?: ReactNode; children: ReactNode }) {
  const reveal = useReveal<HTMLElement>(title)
  return (
    <section ref={reveal} aria-label={`${label} ${title}`} data-detail-pane className="flex min-w-0 scroll-mt-3 flex-col border border-border text-xs">
      <div className="flex min-h-10 min-w-0 flex-wrap items-center gap-2 border-b border-border bg-muted/20 px-3 py-1.5">
        <h2 className="min-w-0 font-mono text-sm break-all">{title}</h2>
        {badges}
        <Button
          variant="ghost"
          size="xs"
          className="ml-auto"
          onClick={() => {
            select(null)
          }}
        >
          <XIcon aria-hidden />
          Close
        </Button>
      </div>
      <div className="flex min-w-0 flex-col gap-3 p-3">
        {actions && <div className="flex min-w-0 flex-wrap gap-2">{actions}</div>}
        {children}
      </div>
    </section>
  )
}

/** A titled part of a detail pane. */
export function Part({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <h3 className={LABEL}>{title}</h3>
      {children}
    </div>
  )
}

/** A list of monospace lines, or `none` when there are none. */
export function Lines({ label, items, none = 'None', render }: { label: string; items: readonly string[]; none?: string; render?: (item: string) => ReactNode }) {
  if (items.length === 0) return <p className="text-muted-foreground">{none}</p>
  return (
    <ul aria-label={label} className="flex min-w-0 flex-col gap-0.5">
      {items.map((i) => (
        <li key={i} className="min-w-0 font-mono break-all">
          {render ? render(i) : i}
        </li>
      ))}
    </ul>
  )
}

/** A tab whose list is empty: the repository has nothing of that kind (spec 063 pattern). */
export function NothingFound({ what, reason }: { what: string; reason: string }) {
  return <EmptyState page={`repository-${what}`} title={`No ${what} found`} reason={reason} guide={{ guide: 'repository' }} />
}

const TONE = {
  warn: 'border-status-warn/50 text-status-warn',
  err: 'border-status-err/50 text-status-err',
  ok: 'border-status-ok/50 text-status-ok',
  muted: 'border-border text-muted-foreground',
} as const

/** A short marker beside a name: "recipients out of date", "external", "unresolved". */
export function Flag({
  tone = 'warn',
  title,
  data,
  children,
}: {
  tone?: keyof typeof TONE
  title?: string | undefined
  data?: Record<`data-${string}`, string>
  children: ReactNode
}) {
  return (
    <span title={title} {...data} className={`border px-1.5 text-xs font-medium whitespace-nowrap ${TONE[tone]}`}>
      {children}
    </span>
  )
}
