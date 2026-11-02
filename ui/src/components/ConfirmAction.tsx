/**
 * Confirmation for a mutating action (T092, constitution I), on the kit's `ConfirmDialog` (spec
 * 065 T029): names every affected object by `namespace/name`, and asks for the count to be typed
 * when there is more than `TYPED_CONFIRM_THRESHOLD`. Esc, Cancel and a click outside close it
 * without sending anything; only the confirm button (or Enter on a matching count) calls
 * `onConfirm`.
 *
 * Test hooks (also used by the live E2E suite): `confirm-dialog`, `confirm-item`,
 * `confirm-typed-count`, `confirm-cancel`, `confirm-submit`. The kit's dialog does not take hooks
 * for its own buttons, list and field, so `TestHooks` tags them once they render.
 */
import { useEffect, useLayoutEffect, useMemo, useRef, type ReactNode } from 'react'

import { ConfirmDialog, type Preview } from '@companion/ui/kit'

import { itemName, TYPED_CONFIRM_THRESHOLD, type ConfirmItem } from './action-utils'

export interface ConfirmActionProps {
  open: boolean
  title: string
  items: readonly ConfirmItem[]
  /** Extra context under the list (what the action does). */
  detail?: ReactNode
  /**
   * Defaults to `items.length > TYPED_CONFIRM_THRESHOLD`. Pass `false` when `items` describes one
   * action rather than counting the objects it mutates — a count is meaningless there.
   */
  requireTypedCount?: boolean
  destructive?: boolean
  /** Confirm button text, e.g. `Delete 3 pods`. */
  confirmLabel: string
  onConfirm: () => void
  /** Esc, Cancel, or a click outside. Nothing is sent. */
  onCancel: () => void
  /**
   * Where focus goes once the dialog has closed. Called with a cancelable event after the kit has
   * returned focus to the opener, so the caller's choice wins, as it did with Radix's own hook.
   */
  onCloseAutoFocus?: (e: Event) => void
}

export function ConfirmAction({
  open,
  title,
  items,
  detail,
  requireTypedCount = items.length > TYPED_CONFIRM_THRESHOLD,
  destructive = false,
  confirmLabel,
  onConfirm,
  onCancel,
  onCloseAutoFocus,
}: ConfirmActionProps) {
  const count = String(items.length)
  const summary = `${
    items.length === 1 ? 'This affects the object below.' : `This affects all ${count} objects below.`
  }${destructive ? ' It cannot be undone.' : ''}`
  const names = items.map(itemName).join('\n')
  // One preview object per opening: the kit clears the typed count when it changes, so a list
  // re-render (a new `items` array with the same names) must not.
  const preview = useMemo<Preview | null>(
    () => (open ? { summary, affected: names === '' ? [] : names.split('\n'), step_up: false } : null),
    [open, summary, names],
  )

  useFocusAfterClose(open, onCloseAutoFocus)

  return (
    <ConfirmDialog
      data-testid="confirm-dialog"
      data-destructive={destructive ? 'true' : undefined}
      preview={preview}
      title={title}
      confirmLabel={confirmLabel}
      tone={destructive ? 'destructive' : 'default'}
      onConfirm={onConfirm}
      onCancel={onCancel}
      {...(requireTypedCount ? { typedConfirm: { word: count } } : {})}
    >
      {detail !== undefined && detail !== null && detail !== false && (
        <div className="text-xs text-muted-foreground">{detail}</div>
      )}
      <TestHooks />
    </ConfirmDialog>
  )
}

/** Hands focus to the caller once the dialog closes (or unmounts while open). */
function useFocusAfterClose(open: boolean, onCloseAutoFocus: ((e: Event) => void) | undefined) {
  const handler = useRef(onCloseAutoFocus)
  useLayoutEffect(() => {
    handler.current = onCloseAutoFocus
  }, [onCloseAutoFocus])
  const wasOpen = useRef(open)
  useEffect(() => {
    const closed = wasOpen.current && !open
    wasOpen.current = open
    if (closed) later(handler.current)
  }, [open])
  useEffect(
    () => () => {
      if (wasOpen.current) later(handler.current)
    },
    [],
  )
}

/** After the kit's own focus return, which runs on the dialog's unmount. */
function later(fn: ((e: Event) => void) | undefined) {
  if (!fn) return
  setTimeout(() => {
    setTimeout(() => {
      fn(new Event('focusoutside', { cancelable: true }))
    }, 0)
  }, 0)
}

/** Tags the kit dialog's list, field and buttons with Fathom's test hooks. */
function TestHooks() {
  const ref = useRef<HTMLSpanElement>(null)
  useLayoutEffect(() => {
    const dialog = ref.current?.closest('[data-slot="dialog-content"]')
    if (!dialog) return
    dialog.querySelector('[data-affected]')?.setAttribute('data-testid', 'confirm-items')
    for (const li of dialog.querySelectorAll('[data-affected] > li')) {
      li.setAttribute('data-testid', 'confirm-item')
    }
    dialog.querySelector('[data-typed-confirm] input')?.setAttribute('data-testid', 'confirm-typed-count')
    const buttons = dialog.querySelectorAll('[data-slot="dialog-footer"] > button')
    buttons[0]?.setAttribute('data-testid', 'confirm-cancel')
    buttons[buttons.length - 1]?.setAttribute('data-testid', 'confirm-submit')
  })
  return <span ref={ref} hidden />
}
