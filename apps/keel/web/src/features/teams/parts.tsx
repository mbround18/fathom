/**
 * Pieces the governance pages share (spec 055 US6 T038): a refusal shown as the server sent it,
 * a dense list pane, and a native select styled like the inputs (dense, keyboard and touch
 * friendly, and what Playwright's `selectOption` drives).
 */
import { WarningOctagonIcon } from '@phosphor-icons/react'
import type { ComponentProps, ReactNode } from 'react'

import type { Refusal } from './store'

/** A `Denied` or `OpError`, verbatim: the action and the reason code, or the server's message. */
export function RefusalText({ refusal }: { refusal: Refusal }) {
  return (
    <p
      role="alert"
      data-refusal={refusal.kind}
      className="flex min-w-0 items-start gap-1.5 border border-status-err/40 px-2 py-1 text-xs text-status-err"
    >
      <WarningOctagonIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
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
    <section data-pane={id} aria-labelledby={`${id}-title`} className="flex min-w-0 flex-col border-b border-border">
      <div className="flex min-w-0 flex-wrap items-center gap-2 px-2 py-1.5">
        <h2 id={`${id}-title`} className="font-display text-xs font-semibold tracking-wider uppercase">
          {title}
        </h2>
        {count !== undefined && count !== null && (
          <span className="text-xs text-muted-foreground tabular-nums">{count}</span>
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
  empty: string
}) {
  if (refusal)
    return (
      <div className="px-2 pb-2">
        <RefusalText refusal={refusal} />
      </div>
    )
  if (items === null)
    return (
      <p role="status" data-pane-state="loading" className="px-2 pb-2 text-xs text-muted-foreground">
        Loading…
      </p>
    )
  if (items.length === 0)
    return (
      <p data-pane-state="empty" className="px-2 pb-2 text-xs text-muted-foreground">
        {empty}
      </p>
    )
  return null
}

export function NativeSelect({ className = '', ...props }: ComponentProps<'select'>) {
  return (
    <select
      {...props}
      className={`h-7 min-w-0 border border-input bg-background px-1 text-xs focus-visible:border-ring focus-visible:outline-none ${className}`}
    />
  )
}

/** A labelled field in an inline form; stacks on narrow screens. */
export function Field({ label, htmlFor, children }: { label: string; htmlFor: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-1 basis-36 flex-col gap-0.5">
      <label htmlFor={htmlFor} className="text-xs text-muted-foreground">
        {label}
      </label>
      {children}
    </div>
  )
}

/** An inline form row under a pane header. */
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
      className="flex min-w-0 flex-wrap items-end gap-2 border-t border-border bg-muted/20 px-2 py-2"
    >
      {children}
    </form>
  )
}
