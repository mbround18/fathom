/**
 * A list of facts about one thing: a term on the left, its value on the right (Keel's run, build
 * and binding panels, Depths' stack detail, Fathom's resource details). Every row is as tall as a
 * Copy button, so rows with and without one keep the same rhythm. `columns={2}` puts two pairs side
 * by side from the `sm` width up; a phone always gets one.
 */
import { cn } from '@companion/theme'
import type { ReactNode } from 'react'

export function DetailList({
  label,
  columns = 1,
  className,
  children,
}: {
  /** The list's accessible name, when the heading above it does not name it. */
  label?: string
  columns?: 1 | 2
  className?: string
  children: ReactNode
}) {
  return (
    <dl
      aria-label={label}
      className={cn(
        'grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-xs',
        columns === 2 && 'sm:grid-cols-[auto_minmax(0,1fr)_auto_minmax(0,1fr)]',
        className,
      )}
    >
      {children}
    </dl>
  )
}

/** One term and its value. `data` is `data-field` on the value, for tests. */
export function DetailRow({ term, children, data }: { term: ReactNode; children: ReactNode; data?: string }) {
  return (
    <>
      <dt className="flex min-h-6 items-center self-start text-muted-foreground">{term}</dt>
      <dd data-field={data} className="flex min-h-6 min-w-0 items-center break-words">
        {children}
      </dd>
    </>
  )
}

/** A value the server did not send, said plainly rather than left blank. */
export function Unknown({ text = 'unknown' }: { text?: string }) {
  return <span className="text-muted-foreground">{text}</span>
}
