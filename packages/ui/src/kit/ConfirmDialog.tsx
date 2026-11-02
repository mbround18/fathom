/**
 * The preview → confirm dialog (spec 060 T053) for a change the server previewed: its summary as
 * sent, what else it affects, whether it asks for a passkey, then Cancel or Confirm. Nothing
 * changes until Confirm; Cancel and Escape send nothing.
 */
import { FingerprintIcon } from '@phosphor-icons/react'
import { Button } from '../button'
import type { ReactNode } from 'react'

import { Dialog } from './Dialog'

export interface Preview {
  summary: string
  affected: readonly string[]
  step_up: boolean
}

/** What else a change affects, and whether it needs a passkey. */
export function PreviewDetails({
  preview,
  stepUp = 'This change asks for your passkey.',
}: {
  preview: Preview
  /** The step-up line's wording. */
  stepUp?: string
}) {
  return (
    <>
      {preview.affected.length > 0 && (
        <div className="flex min-w-0 flex-col gap-1">
          <h3 className="text-sm font-semibold">
            Affects {String(preview.affected.length)}
          </h3>
          <ul aria-label="Affected" data-affected className="max-h-48 min-w-0 overflow-y-auto border border-border text-xs">
            {preview.affected.map((a, i) => (
              <li key={i} className="border-t border-border px-2 py-1 font-mono break-all first:border-t-0">
                {a}
              </li>
            ))}
          </ul>
        </div>
      )}
      {preview.step_up && (
        <p data-step-up className="flex items-center gap-1.5 text-xs text-status-warn">
          <FingerprintIcon aria-hidden className="size-3.5 shrink-0" />
          {stepUp}
        </p>
      )}
    </>
  )
}

export function ConfirmDialog({
  preview,
  title = 'Confirm',
  confirmLabel = 'Confirm',
  onConfirm,
  onCancel,
  busy = false,
  busyLabel,
  tone = 'default',
  children,
  ...rest
}: {
  preview: Preview | null
  title?: string
  confirmLabel?: string
  onConfirm: () => void
  onCancel: () => void
  busy?: boolean
  /** What the confirm button says while `busy` (it keeps `confirmLabel` otherwise). */
  busyLabel?: string
  /** `destructive` for a change that removes, cancels or denies: the confirm button says so. */
  tone?: 'default' | 'destructive'
  children?: ReactNode
  [data: `data-${string}`]: string | boolean | undefined
}) {
  return (
    <Dialog
      {...rest}
      open={preview !== null}
      onClose={onCancel}
      title={title}
      description={preview ? <span data-preview-summary>{preview.summary}</span> : undefined}
      footer={
        <>
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          <Button variant={tone === 'destructive' ? 'destructive' : 'default'} disabled={busy} onClick={onConfirm}>
            {preview?.step_up && <FingerprintIcon aria-hidden />}
            {busy && busyLabel ? busyLabel : confirmLabel}
          </Button>
        </>
      }
    >
      {preview && (
        <>
          <PreviewDetails preview={preview} />
          {children}
        </>
      )}
    </Dialog>
  )
}
