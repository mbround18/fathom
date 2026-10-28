/**
 * Nothing here yet (spec 060 R10): what is missing, why it matters, and the one action that fixes
 * it.
 */
import type { ReactNode } from 'react'

export function EmptyState({
  icon,
  title,
  children,
  action,
}: {
  icon?: ReactNode
  title: string
  children?: ReactNode
  action?: ReactNode
}) {
  return (
    <div
      data-pane-state="empty"
      className="flex min-w-0 flex-col items-start gap-2 border border-dashed border-border px-4 py-6 sm:items-center sm:text-center"
    >
      {icon !== undefined && <span className="text-muted-foreground [&_svg]:size-6">{icon}</span>}
      <p className="font-display text-xs font-semibold tracking-wider uppercase">{title}</p>
      {children !== undefined && <div className="max-w-prose text-xs text-muted-foreground">{children}</div>}
      {action !== undefined && <div className="pt-1">{action}</div>}
    </div>
  )
}
