/**
 * A page's header (spec 060 R10): the title, an optional one-line description, and actions on the
 * right that wrap under the title on a phone.
 *
 * With `guide` (spec 063 R8, FR-023) the description is held to one line on desktop and followed by
 * "Learn more": the longer explanation lives in that guide, not on the page. Write the ref as a
 * literal, `guide={{ guide: 'hosts' }}`, so `guide-links.test.ts` can check it. `tip` sits right
 * after the title, outside the `h1`, for a glossary term's `TermTip`.
 *
 * `size="pane"` (spec 065 R9) is a pane's compact one-row header instead: `lead` (a slot number, a
 * slot menu), the title as an `h2`, an optional `count`, then `actions` on the right. `rootProps`
 * go on the `header`, for a pane's drag handlers. A title that must not be upper-cased wraps itself
 * in a `normal-case` span.
 */
import type { HTMLAttributes, ReactNode } from 'react'

import { H1 } from './form'
import { GuideLink } from './GuideLink'
import type { GuideRef } from './provider'

interface PageProps {
  size?: 'page'
  /** The heading's id, for the page's `aria-labelledby`. */
  id: string
  title: ReactNode
  /** An info icon for the title's term, outside the heading. */
  tip?: ReactNode
  description?: ReactNode
  /** Where the description continues: a "Learn more" link after it. */
  guide?: GuideRef
  actions?: ReactNode
  /** A toolbar row under the title (pickers, filters). */
  children?: ReactNode
}

interface PaneProps {
  size: 'pane'
  /** The heading's id, for the pane's `aria-labelledby`. */
  id?: string
  title: ReactNode
  tip?: ReactNode
  /** Before the title: a slot number, a slot menu. */
  lead?: ReactNode
  /** After the title, muted: how many items the pane holds. */
  count?: ReactNode
  actions?: ReactNode
  /** On the `header` element: drag handlers, `data-*` markers. */
  rootProps?: HTMLAttributes<HTMLElement> & { [data: `data-${string}`]: string | undefined }
}

export function PageHeader(props: PageProps | PaneProps) {
  if (props.size === 'pane') return <PaneHeader {...props} />
  const { id, title, tip, description, guide, actions, children } = props
  return (
    <header className="flex min-w-0 flex-col gap-3 border-b border-border pb-4">
      <div className="flex min-w-0 flex-wrap items-start gap-x-3 gap-y-2">
        <div className="flex min-w-0 flex-1 basis-48 flex-col gap-1">
          <div className="flex min-w-0 items-center gap-1.5">
            <h1 id={id} className={H1}>
              {title}
            </h1>
            {tip}
          </div>
          {description !== undefined &&
            (guide === undefined ? (
              <p className="max-w-prose text-sm text-muted-foreground">{description}</p>
            ) : (
              <div data-page-intro className="flex max-w-prose min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
                <p className="min-w-0 text-sm text-muted-foreground md:line-clamp-1">{description}</p>
                <GuideLink to={guide} />
              </div>
            ))}
        </div>
        {actions !== undefined && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
      </div>
      {children !== undefined && <div className="flex min-w-0 flex-wrap items-center gap-2">{children}</div>}
    </header>
  )
}

function PaneHeader({ id, title, tip, lead, count, actions, rootProps }: PaneProps) {
  return (
    <header
      {...rootProps}
      data-pane-header=""
      className="flex h-[max(2rem,var(--touch-target))] min-w-0 shrink-0 items-center gap-2 border-b border-border bg-card px-2"
    >
      {lead}
      <h2 id={id} className="min-w-0 truncate font-display text-xs font-semibold tracking-wider uppercase">
        {title}
      </h2>
      {tip}
      {count !== undefined && (
        <span data-pane-count className="shrink-0 text-xs text-muted-foreground tabular-nums">
          {count}
        </span>
      )}
      {actions !== undefined && <div className="ml-auto flex shrink-0 items-center gap-1">{actions}</div>}
    </header>
  )
}
