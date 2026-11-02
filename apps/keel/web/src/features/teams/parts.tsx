/**
 * Pieces the governance pages share (spec 055 US6 T038): a dense list pane, its loading, failed or
 * empty state, and a native select styled like the inputs (dense, keyboard and touch
 * friendly, and what Playwright's `selectOption` drives).
 */
import type { ComponentProps, ReactNode } from 'react'
import { FORM_WIDTH, H2, LABEL, LoadFailed, LoadState, Select } from '@companion/ui/kit'

import type { Refusal } from '@/app/load'

export { RefusalText } from '@companion/ui/kit'

/** A titled list pane: a header row with actions, then the body. */
export function Pane({
  id,
  title,
  tip,
  count,
  actions,
  children,
}: {
  id: string
  title: string
  /** An info icon for the title's term, outside the heading. */
  tip?: ReactNode
  count?: number | null | undefined
  actions?: ReactNode | undefined
  children: ReactNode
}) {
  return (
    <section data-pane={id} aria-labelledby={`${id}-title`} className="flex min-w-0 flex-col border border-border">
      <div className="flex min-h-12 min-w-0 flex-wrap items-center gap-x-3 gap-y-2 border-b border-border px-4 py-2">
        <h2 id={`${id}-title`} className={H2}>
          {title}
        </h2>
        {tip}
        {count !== undefined && count !== null && (
          <span data-pane-count className="text-sm text-muted-foreground tabular-nums">{count}</span>
        )}
        <span className="flex-1" />
        {actions}
      </div>
      {children}
    </section>
  )
}

/** Loading, failed or empty, for a list that has nothing to show yet. */
export function ListState({
  items,
  refusal,
  what,
  onRetry,
  empty,
}: {
  items: readonly unknown[] | null
  refusal: Refusal | undefined
  /** The list, for "Failed to load {what}." */
  what: string
  onRetry: () => void
  /** A line, or a guided `EmptyState`. */
  empty: ReactNode
}) {
  if (refusal)
    return (
      <div className="p-4">
        <LoadFailed what={what} refusal={refusal} onRetry={onRetry} />
      </div>
    )
  if (items === null)
    return (
      <div className="px-4">
        <LoadState loading />
      </div>
    )
  if (items.length === 0)
    return typeof empty === 'string' ? (
      <p data-pane-state="empty" className="m-4 max-w-prose text-sm text-muted-foreground">
        {empty}
      </p>
    ) : (
      <div className="m-4">{empty}</div>
    )
  return null
}

/** A native select, a little shorter than a form field so it fits in a dense row. */
export function NativeSelect({ className = '', ...props }: ComponentProps<'select'>) {
  return <Select {...props} dense className={className} />
}

/** A labelled field in an inline form; stacks on narrow screens and stops growing at a name's width. */
export function Field({ label, htmlFor, children }: { label: string; htmlFor: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-1 basis-48 flex-col gap-1 sm:max-w-72">
      <label htmlFor={htmlFor} className={LABEL}>
        {label}
      </label>
      {children}
    </div>
  )
}

/** An inline form under a pane header: no band of its own, just the fields and their button. */
export function InlineForm({
  label,
  onSubmit,
  children,
}: {
  label: string
  onSubmit: () => void
  children: ReactNode
}) {
  return (
    <form
      aria-label={label}
      onSubmit={(e) => {
        e.preventDefault()
        onSubmit()
      }}
      className={`flex min-w-0 flex-wrap items-end gap-3 px-4 py-4 ${FORM_WIDTH}`}
    >
      {children}
    </form>
  )
}
