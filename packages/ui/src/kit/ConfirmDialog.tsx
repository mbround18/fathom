/**
 * The preview → confirm dialog (spec 060 T053) for a change the server previewed: its summary as
 * sent, what else it affects, whether it asks for a passkey, then Cancel or Confirm. Nothing
 * changes until Confirm; Cancel and Escape send nothing.
 *
 * `typedConfirm` (spec 065 R9) is for a change too large to confirm with one click: Confirm stays
 * off until the user types `word`, and Enter in that field confirms once it matches. The field
 * starts empty each time the dialog opens.
 */
import { FingerprintIcon } from '@phosphor-icons/react'
import { Button } from '../button'
import { Input } from '../input'
import { useId, useState, type ReactNode } from 'react'

import { Dialog } from './Dialog'
import { LABEL } from './form'

export interface Preview {
  summary: string
  affected: readonly string[]
  step_up: boolean
  /** Things worth knowing before confirming that don't stop the change. */
  warnings?: readonly string[]
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
      {preview.warnings && preview.warnings.length > 0 && (
        <ul aria-label="Warnings" data-preview-warnings className="flex min-w-0 flex-col gap-1 text-xs text-status-warn">
          {preview.warnings.map((w, i) => (
            <li key={i} className="break-words">
              {w}
            </li>
          ))}
        </ul>
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
  typedConfirm,
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
  /** Confirm only after the user types `word` (a count, a name). */
  typedConfirm?: { word: string }
  children?: ReactNode
  [data: `data-${string}`]: string | boolean | undefined
}) {
  const typedId = useId()
  // The text belongs to the preview it was typed for, so a new opening starts empty.
  const [typed, setTyped] = useState<{ for: Preview | null; text: string }>({ for: null, text: '' })
  // Closing forgets it too, for an app that reopens with the same preview object.
  if (preview === null && typed.for !== null) setTyped({ for: null, text: '' })
  const text = typed.for === preview ? typed.text : ''
  const armed = typedConfirm === undefined || text.trim() === typedConfirm.word
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
          <Button variant={tone === 'destructive' ? 'destructive' : 'default'} disabled={busy || !armed} onClick={onConfirm}>
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
          {typedConfirm && (
            <div data-typed-confirm className="flex min-w-0 flex-col gap-1">
              <label htmlFor={typedId} className={LABEL}>
                Type <strong className="font-mono text-foreground">{typedConfirm.word}</strong> to confirm
              </label>
              <Input
                id={typedId}
                autoComplete="off"
                spellCheck={false}
                value={text}
                onChange={(e) => {
                  setTyped({ for: preview, text: e.target.value })
                }}
                onKeyDown={(e) => {
                  if (e.key !== 'Enter') return
                  e.preventDefault()
                  if (armed && !busy) onConfirm()
                }}
                className="font-mono"
              />
            </div>
          )}
        </>
      )}
    </Dialog>
  )
}
