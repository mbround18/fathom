/**
 * The app's dialog (spec 060 R10): `@companion/ui`'s radix dialog with a title, an optional
 * description, a scrolling body and a footer. On a phone it fills the width less a gutter and
 * scrolls inside itself, so nothing scrolls the page sideways.
 *
 * `onClose` runs for Escape, the close button and a click outside; a dialog that must warn before
 * closing (the enrolment token) decides there whether to close.
 */
import {
  Dialog as Root,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@companion/ui'
import type { ReactNode } from 'react'

const WIDTH = { sm: 'sm:max-w-sm', md: 'sm:max-w-md', lg: 'sm:max-w-xl' } as const

export function Dialog({
  open,
  onClose,
  title,
  description,
  footer,
  size = 'md',
  children,
  ...rest
}: {
  open: boolean
  onClose: () => void
  title: ReactNode
  description?: ReactNode
  footer?: ReactNode
  size?: keyof typeof WIDTH
  children?: ReactNode
  [data: `data-${string}`]: string | boolean | undefined
}) {
  return (
    <Root
      open={open}
      onOpenChange={(o) => {
        if (!o) onClose()
      }}
    >
      <DialogContent
        {...rest}
        className={`flex max-h-[calc(100dvh-2rem)] max-w-[calc(100%-1rem)] min-w-0 flex-col gap-3 overflow-hidden ${WIDTH[size]}`}
      >
        <DialogHeader className="pr-8">
          <DialogTitle className="font-display text-lg font-semibold">{title}</DialogTitle>
          {description !== undefined ? (
            <DialogDescription data-summary>{description}</DialogDescription>
          ) : (
            <DialogDescription className="sr-only">{title}</DialogDescription>
          )}
        </DialogHeader>
        {children !== undefined && (
          <div className="-mx-4 flex min-h-0 min-w-0 flex-1 flex-col gap-3 overflow-y-auto px-4">{children}</div>
        )}
        {footer !== undefined && <DialogFooter>{footer}</DialogFooter>}
      </DialogContent>
    </Root>
  )
}
