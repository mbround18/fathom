/**
 * Nothing here yet (spec 060 R10): what is missing, why it matters, and the one action that fixes
 * it.
 *
 * A page's empty main list is guided (spec 063 R2, FR-011): a one-line `reason`, two or three
 * numbered `steps`, the page's one primary `action`, and a `guide` link under it. Someone who
 * can't take the action gets `whoCan` instead, as "{whoCan} can {verb}." (FR-012): never both, and
 * never a disabled button. No illustrations (FR-017). `page` sets `data-empty-page` and marks the
 * root `data-empty`, so callers from before 063 render as they did.
 */
import type { ReactNode } from 'react'

import type { GuideRef } from '@/help/guides'

import { GuideLink } from './GuideLink'

export function EmptyState({
  icon,
  title,
  reason,
  steps,
  action,
  whoCan,
  verb,
  guide,
  page,
  children,
}: {
  icon?: ReactNode
  title: string
  /** One line on why it matters. */
  reason?: ReactNode
  /** Two or three next steps, shown numbered. */
  steps?: ReactNode[] | undefined
  /** The page's one primary button. */
  action?: ReactNode
  /** Who can act, when the person can't ("An owner or admin"); ignored with `action`. */
  whoCan?: string | undefined
  /** What they can do ("create receivers"). */
  verb?: string | undefined
  guide?: GuideRef | undefined
  /** `data-empty-page`: which page's main list this is. */
  page?: string
  children?: ReactNode
}) {
  const restricted = action === undefined && whoCan !== undefined
  const variant = page === undefined ? undefined : restricted ? 'restricted' : 'guided'
  return (
    <div
      data-pane-state="empty"
      data-empty={variant}
      data-empty-page={page}
      className="flex min-w-0 flex-col items-start gap-2 border border-dashed border-border px-4 py-6 sm:items-center sm:text-center"
    >
      {icon !== undefined && <span className="text-muted-foreground [&_svg]:size-6">{icon}</span>}
      <p className="font-display text-base font-semibold">{title}</p>
      {reason !== undefined && <p className="max-w-prose text-sm text-muted-foreground">{reason}</p>}
      {children !== undefined && <div className="max-w-prose text-sm text-muted-foreground">{children}</div>}
      {steps !== undefined && steps.length > 0 && (
        <ol className="flex max-w-prose list-decimal flex-col gap-1 pl-5 text-left text-sm marker:text-muted-foreground marker:tabular-nums">
          {steps.map((s, i) => (
            <li key={i} className="pl-1">
              {s}
            </li>
          ))}
        </ol>
      )}
      {action !== undefined && (
        <div data-empty-action className="pt-1">
          {action}
        </div>
      )}
      {restricted && (
        <p data-who-can className="max-w-prose text-sm text-muted-foreground">
          {whoCan} can {verb}.
        </p>
      )}
      {guide !== undefined && (
        <GuideLink to={guide}>
          {/* The visible text is the contract's "Learn more"; the hidden title makes the link's
              name make sense in a screen reader's list of links. */}
          <span aria-hidden>Learn more</span>
          <span className="sr-only">Learn more: {title}</span>
        </GuideLink>
      )}
    </div>
  )
}
