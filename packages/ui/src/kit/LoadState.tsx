/**
 * Loading, failed or loaded (spec 060 R10). While `loading`, a quiet status line; when `failed`,
 * `LoadFailed` instead, naming `what` and offering `onRetry`; otherwise the children.
 */
import { CircleNotchIcon } from '@phosphor-icons/react'
import type { ReactNode } from 'react'

import type { Refusal } from './refusal'

import { LoadFailed } from './LoadFailed'

export function LoadState({
  loading,
  failed,
  what = 'this',
  onRetry,
  label = 'Loading…',
  children,
}: {
  loading: boolean
  failed?: Refusal | null | undefined
  /** What failed to load, for "Failed to load {what}." */
  what?: string
  onRetry?: (() => void) | undefined
  label?: string
  children?: ReactNode
}) {
  if (failed) return <LoadFailed what={what} refusal={failed} onRetry={onRetry} />
  if (loading)
    return (
      <p role="status" data-pane-state="loading" className="flex items-center gap-1.5 py-2 text-xs text-muted-foreground">
        <CircleNotchIcon aria-hidden className="size-3.5 animate-spin motion-reduce:animate-none" />
        {label}
      </p>
    )
  return <>{children}</>
}
