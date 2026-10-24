import { WarningIcon } from '@phosphor-icons/react'
import { memo } from 'react'

import { Link } from '@/app/Link'
import type { ForeignProject, Stack } from '@/protocol/generated'

import { blockedReason, formatTime, stackHref, stackNotes, type StackNote } from './model'
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
 * Blocked stacks (invalid, unreadable) say why and carry no actions.
 */
export const StackRow = memo(function StackRow({ stack, elsewhere }: { stack: Stack; elsewhere: readonly ForeignProject[] }) {
  const blocked = blockedReason(stack) !== null
  return (
    <article
      data-stack={stack.name}
      data-blocked={blocked || undefined}
      aria-label={stack.name}
      className="flex min-w-0 flex-col gap-1 border-b border-border px-2 py-1.5 hover:bg-muted/40"
    >
      <div className="flex min-w-0 items-center gap-2">
        <Link
          to={stackHref(stack.name)}
          className="min-w-0 flex-1 truncate text-sm font-medium text-foreground hover:text-primary focus-visible:text-primary"
        >
          {stack.name}
        </Link>
        <StatusBadge status={stack.status} unhealthy={stack.unhealthy} />
      </div>
      <div className="flex min-w-0 flex-wrap gap-x-3 text-xs text-muted-foreground">
        <span data-field="services">
          {stack.running_services}/{stack.total_services} services up
        </span>
        <span data-field="modified">changed {formatTime(stack.modified_at)}</span>
      </div>
      <StackNotes notes={stackNotes(stack, elsewhere)} />
    </article>
  )
})
