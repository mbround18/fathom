/**
 * A page's header (spec 060 R10): the title, an optional one-line description, and actions on the
 * right that wrap under the title on a phone.
 */
import type { ReactNode } from 'react'

export function PageHeader({
  id,
  title,
  description,
  actions,
  children,
}: {
  /** The heading's id, for the page's `aria-labelledby`. */
  id: string
  title: ReactNode
  description?: ReactNode
  actions?: ReactNode
  /** A toolbar row under the title (pickers, filters). */
  children?: ReactNode
}) {
  return (
    <header className="flex min-w-0 flex-col gap-2 border-b border-border pb-3">
      <div className="flex min-w-0 flex-wrap items-start gap-x-3 gap-y-2">
        <div className="flex min-w-0 flex-1 basis-48 flex-col gap-0.5">
          <h1 id={id} className="font-display text-sm font-semibold tracking-wider uppercase">
            {title}
          </h1>
          {description !== undefined && <p className="text-xs text-muted-foreground">{description}</p>}
        </div>
        {actions !== undefined && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
      </div>
      {children !== undefined && <div className="flex min-w-0 flex-wrap items-center gap-2">{children}</div>}
    </header>
  )
}
