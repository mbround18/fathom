import { WarningIcon } from '@phosphor-icons/react'
import { memo } from 'react'

import { Link } from '@/app/Link'
import type { ForeignProject, Stack } from '@/protocol/generated'

import { blockedReason, formatTime, stackHref, stackNotes, type StackNote } from './model'
import { stackReasons } from './security'
import { SecurityChip } from './SecurityChip'
import { StatusBadge } from './StatusBadge'

export function StackNotes({ notes }: { notes: readonly StackNote[] }) {
  if (notes.length === 0) return null
  return (
    <ul className="flex flex-col gap-0.5">
      {notes.map((n) => (
        <li
          key={n.key}
          data-note={n.key}
          className={`flex min-w-0 items-start gap-1 text-xs break-words ${n.tone === 'err' ? 'text-status-err' : 'text-status-warn'}`}
        >
          <WarningIcon aria-hidden className="mt-0.5 size-3 shrink-0" />
          <span className="min-w-0">{n.text}</span>
        </li>
      ))}
    </ul>
  )
}

/**
 * One stack in the list: name, status (colour and text), running/total services, last change.
 * Blocked stacks (invalid, unreadable) say why and carry no actions. Two lines in a narrow list,
 * one line once the list is wide enough (a container query, so the sidebar stays two lines).
 * The whole row opens the stack: the name link stretches over it, and the chips sit above.
 */
export const StackRow = memo(function StackRow({
  stack,
  elsewhere,
  selected = false,
}: {
  stack: Stack
  elsewhere: readonly ForeignProject[]
  selected?: boolean
}) {
  const blocked = blockedReason(stack) !== null
  return (
    <article
      data-stack={stack.name}
      data-blocked={blocked || undefined}
      data-selected={selected || undefined}
      aria-label={stack.name}
      className="relative flex min-w-0 cursor-pointer flex-col gap-1 border-b border-l-2 border-b-border border-l-transparent px-2 py-1.5 hover:bg-muted/40 data-selected:border-l-primary data-selected:bg-muted/60"
    >
      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-2 gap-y-1 @2xl:grid-cols-[minmax(0,1fr)_auto_auto] @2xl:gap-x-4">
        <Link
          to={stackHref(stack.name)}
          aria-current={selected ? 'page' : undefined}
          className="min-w-0 truncate text-sm font-medium text-foreground after:absolute after:inset-0 hover:text-primary focus-visible:text-primary aria-[current=page]:text-primary"
        >
          {stack.name}
        </Link>
        <div className="col-span-2 row-start-2 flex min-w-0 flex-wrap gap-x-3 text-xs text-muted-foreground @2xl:col-span-1 @2xl:col-start-2 @2xl:row-start-1">
          <span data-field="services" className="tabular-nums">
            {stack.running_services}/{stack.total_services} services up
          </span>
          <span data-field="modified">changed {formatTime(stack.modified_at)}</span>
        </div>
        <span className="relative z-10 col-start-2 row-start-1 flex items-center gap-1 justify-self-end @2xl:col-start-3">
          <SecurityChip level={stack.security} reasons={stackReasons(stack)} />
          <StatusBadge status={stack.status} unhealthy={stack.unhealthy} />
        </span>
      </div>
      <StackNotes notes={stackNotes(stack, elsewhere)} />
    </article>
  )
})
