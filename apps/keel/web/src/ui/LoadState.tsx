/**
 * Loading, failed or loaded (spec 060 R10). While `loading`, a quiet status line; with `error`,
 * that instead (a refusal is shown as the server sent it); otherwise the children.
 */
import { CircleNotchIcon } from '@phosphor-icons/react'
import type { ReactNode } from 'react'

export function LoadState({
  loading,
  error,
  label = 'Loading…',
  children,
}: {
  loading: boolean
  error?: ReactNode
  label?: string
  children?: ReactNode
}) {
  if (error !== undefined && error !== null && error !== false) return <>{error}</>
  if (loading)
    return (
      <p role="status" data-pane-state="loading" className="flex items-center gap-1.5 py-2 text-xs text-muted-foreground">
        <CircleNotchIcon aria-hidden className="size-3.5 animate-spin motion-reduce:animate-none" />
        {label}
      </p>
    )
  return <>{children}</>
}
