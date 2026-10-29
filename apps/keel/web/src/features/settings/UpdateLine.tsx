/**
 * The admin's update line (spec 061 FR-015, SC-006): read only, and nothing about the available
 * image. Admins get `UpdateStatusLine`, which carries nothing else; when there is no update, there
 * is no line.
 */
import { ArrowCircleUpIcon } from '@phosphor-icons/react'

import { adminLine, useUpdates } from './updates'

export function UpdateLine() {
  const text = useUpdates((s) => adminLine(s.line))
  if (text === null) return null
  return (
    <p role="status" data-update-line className="flex items-center gap-1.5 border border-border px-3 py-2 text-xs text-muted-foreground">
      <ArrowCircleUpIcon aria-hidden className="size-3.5 shrink-0 text-accent-neon" />
      {text}
    </p>
  )
}
