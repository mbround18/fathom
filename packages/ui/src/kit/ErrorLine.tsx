/**
 * An error the person can dismiss (spec 060 T053): shown as the server sent it, in one place on
 * the page, announced to screen readers.
 */
import { WarningOctagonIcon, XIcon } from '@phosphor-icons/react'
import { Button } from '../button'

export function ErrorLine({ message, onDismiss }: { message: string | null | undefined; onDismiss?: () => void }) {
  if (!message) return null
  return (
    <div
      role="alert"
      className="flex min-w-0 items-start gap-2 border border-status-err/40 bg-status-err/5 px-2 py-1.5 text-xs text-status-err"
    >
      <WarningOctagonIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
      <span className="min-w-0 flex-1 break-words whitespace-pre-wrap">{message}</span>
      {onDismiss && (
        <Button variant="ghost" size="xs" className="-my-0.5 shrink-0" onClick={onDismiss}>
          <XIcon aria-hidden />
          Dismiss
        </Button>
      )}
    </div>
  )
}
