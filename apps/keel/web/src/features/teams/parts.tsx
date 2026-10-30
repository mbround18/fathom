/**
 * Pieces the governance pages share (spec 055 US6 T038): a refusal shown as the server sent it,
 * a dense list pane, and a native select styled like the inputs (dense, keyboard and touch
 * friendly, and what Playwright's `selectOption` drives).
 */
import { WarningOctagonIcon } from '@phosphor-icons/react'
import type { ComponentProps, ReactNode } from 'react'

import { FORM_WIDTH, H2, LABEL, Select } from '@/ui/form'
import { LoadState } from '@/ui/LoadState'

import type { Refusal } from './store'

/** A `Denied` or `OpError`, verbatim: the action and the reason code, or the server's message. */
export function RefusalText({ refusal }: { refusal: Refusal }) {
  return (
    <p
      role="alert"
      data-refusal={refusal.kind}
      className="flex min-w-0 items-start gap-1.5 border border-status-err/40 px-2 py-1.5 text-sm text-status-err"
    >
      <WarningOctagonIcon aria-hidden className="mt-0.5 size-4 shrink-0" />
      {refusal.kind === 'denied' ? (
        <span className="min-w-0 break-words">
          Denied: <code data-denied-action>{refusal.action}</code> (
          <code data-denied-reason>{refusal.reason}</code>)
        </span>
      ) : (
        <span className="min-w-0 break-words">
          <span className="whitespace-pre-wrap">{refusal.message}</span> (<code data-error-code>{refusal.code}</code>)
        </span>
      )}
    </p>
  )
}

/** A titled list pane: a header row with actions, then the body. */
export function Pane({
  id,
  title,
  count,
  actions,
  children,
}: {
  id: string
  title: string
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

/** Loading, refused or empty, for a list that has nothing to show yet. */
export function ListState({
  items,
  refusal,
  empty,
}: {
  items: readonly unknown[] | null
  refusal: Refusal | undefined
  /** A line, or a guided `EmptyState`. */
  empty: ReactNode
}) {
  if (refusal)
    return (
      <div className="p-4">
        <RefusalText refusal={refusal} />
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
