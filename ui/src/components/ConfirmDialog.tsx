/**
 * Confirmation for a mutating action (T092, constitution I): names every affected object by
 * `namespace/name`, and asks for the count to be typed when there are more than five. Esc and
 * Cancel close it without sending anything; only the confirm button calls `onConfirm`.
 *
 * Test hooks (also used by the live E2E suite): `confirm-dialog`, `confirm-item`,
 * `confirm-typed-count`, `confirm-cancel`, `confirm-submit`.
 */
import { WarningIcon } from '@phosphor-icons/react'
import { useId, useState, type ReactNode } from 'react'

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'

import { itemName, TYPED_CONFIRM_THRESHOLD, type ConfirmItem } from './action-utils'

export interface ConfirmDialogProps {
  open: boolean
  title: string
  items: readonly ConfirmItem[]
  /** Extra context under the list (what the action does). */
  detail?: ReactNode
  /** Defaults to `items.length > TYPED_CONFIRM_THRESHOLD`. */
  requireTypedCount?: boolean
  destructive?: boolean
  /** Confirm button text, e.g. `Delete 3 pods`. */
  confirmLabel: string
  onConfirm: () => void
  /** Esc, Cancel, or a click outside. Nothing is sent. */
  onCancel: () => void
  /** Where focus goes on close (Radix `onCloseAutoFocus`). */
  onCloseAutoFocus?: (e: Event) => void
}

export function ConfirmDialog(props: ConfirmDialogProps) {
  const { open, onCancel, onCloseAutoFocus } = props
  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onCancel()
      }}
    >
      {open && (
        <AlertDialogContent
          data-testid="confirm-dialog"
          data-destructive={props.destructive ? 'true' : undefined}
          onCloseAutoFocus={onCloseAutoFocus}
          className="flex max-h-[min(90dvh,40rem)] w-[calc(100vw-2rem)] flex-col gap-3 data-[size=default]:max-w-lg data-[size=default]:sm:max-w-lg"
        >
          <ConfirmBody {...props} />
        </AlertDialogContent>
      )}
    </AlertDialog>
  )
}

/** Mounted per open, so the typed count resets every time. */
function ConfirmBody({
  title,
  items,
  detail,
  requireTypedCount = items.length > TYPED_CONFIRM_THRESHOLD,
  destructive = false,
  confirmLabel,
  onConfirm,
}: ConfirmDialogProps) {
  const [typed, setTyped] = useState('')
  const inputId = useId()
  const count = String(items.length)
  const armed = !requireTypedCount || typed.trim() === count

  return (
    <form
      className="contents"
      onSubmit={(e) => {
        e.preventDefault()
        if (armed) onConfirm()
      }}
    >
      <AlertDialogHeader className="place-items-start text-left">
        <AlertDialogTitle
          className={cn(
            'flex items-center gap-2 font-display text-base font-semibold tracking-wider uppercase',
            destructive ? 'text-destructive' : 'text-accent-neon',
          )}
        >
          <WarningIcon aria-hidden="true" className="size-5 shrink-0" weight="bold" />
          {title}
        </AlertDialogTitle>
        <AlertDialogDescription className="text-left text-xs text-muted-foreground">
          {items.length === 1
            ? 'This affects the object below.'
            : `This affects all ${count} objects below.`}{' '}
          {destructive ? 'It cannot be undone.' : ''}
        </AlertDialogDescription>
      </AlertDialogHeader>

      <ul
        aria-label="Affected objects"
        data-testid="confirm-items"
        className="min-h-0 flex-1 overflow-y-auto border border-border bg-background font-mono text-sm"
      >
        {items.map((i) => (
          <li
            key={itemName(i)}
            data-testid="confirm-item"
            className="flex min-w-0 items-baseline gap-2 border-b border-border/40 px-2 py-1.5 last:border-b-0"
          >
            <span
              aria-hidden="true"
              className={destructive ? 'text-destructive' : 'text-accent-neon'}
            >
              {destructive ? '✖' : '▸'}
            </span>
            <span className="min-w-0 break-all">
              {i.namespace && <span className="text-muted-foreground">{i.namespace}/</span>}
              <span className="text-foreground">{i.name}</span>
            </span>
          </li>
        ))}
      </ul>

      {detail && <div className="text-xs text-muted-foreground">{detail}</div>}

      {requireTypedCount && (
        <div className="flex flex-col gap-1.5">
          <label htmlFor={inputId} className="text-xs">
            Type <strong className="font-mono text-foreground">{count}</strong> to confirm
          </label>
          <Input
            id={inputId}
            data-testid="confirm-typed-count"
            inputMode="numeric"
            autoComplete="off"
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            className="h-[max(2rem,var(--touch-target))] font-mono"
          />
        </div>
      )}

      <AlertDialogFooter className="gap-2">
        <AlertDialogCancel
          data-testid="confirm-cancel"
          className="h-[max(2rem,var(--touch-target))] min-w-24"
        >
          Cancel
        </AlertDialogCancel>
        <Button
          type="submit"
          data-testid="confirm-submit"
          variant={destructive ? 'destructive' : 'default'}
          disabled={!armed}
          className={cn(
            'h-[max(2rem,var(--touch-target))] min-w-24',
            destructive &&
              'border-destructive bg-destructive text-white hover:bg-destructive/85 dark:bg-destructive dark:hover:bg-destructive/85',
          )}
        >
          {confirmLabel}
        </Button>
      </AlertDialogFooter>
    </form>
  )
}
