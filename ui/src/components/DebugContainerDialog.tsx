/**
 * Add-debug-container dialog (025-ephemeral-debug-containers): collects an image and the target
 * container, and shows the pod, target, image and the irreversibility warning before the confirm
 * control is enabled (FR-013). Esc and Cancel send nothing.
 *
 * Test hooks: `debug-dialog`, `debug-target-pod`, `debug-image`, `debug-target-container`,
 * `debug-summary`, `debug-warning`, `debug-cancel`, `debug-submit`.
 */
import { BugIcon, WarningIcon } from '@phosphor-icons/react'
import { useId, useState } from 'react'

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  Input,
} from '@companion/ui'

import { debugInputValid, EPHEMERAL_WARNING, itemName, type ConfirmItem } from './action-utils'

export interface DebugContainerDialogProps {
  open: boolean
  item: ConfirmItem
  /** The pod's existing (non-ephemeral) container names; one entry is auto-selected. */
  containers: readonly string[]
  onConfirm: (input: { image: string; targetContainer: string }) => void
  onCancel: () => void
  onCloseAutoFocus?: (e: Event) => void
}

export function DebugContainerDialog(props: DebugContainerDialogProps) {
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
          data-testid="debug-dialog"
          data-destructive="true"
          onCloseAutoFocus={onCloseAutoFocus}
          onOpenAutoFocus={(e) => {
            const input = (e.currentTarget as HTMLElement | null)?.querySelector('input')
            if (input) {
              e.preventDefault()
              input.focus()
            }
          }}
          className="flex max-h-[min(90dvh,40rem)] w-[calc(100vw-2rem)] flex-col gap-3 overflow-y-auto data-[size=default]:max-w-md data-[size=default]:sm:max-w-md"
        >
          <DebugBody {...props} />
        </AlertDialogContent>
      )}
    </AlertDialog>
  )
}

function DebugBody({ item, containers, onConfirm }: DebugContainerDialogProps) {
  const [image, setImage] = useState('')
  const [target, setTarget] = useState(containers.length === 1 ? (containers[0] ?? '') : '')
  const imageId = useId()
  const targetId = useId()
  const listId = useId()
  const valid = debugInputValid(image, target)

  return (
    <form
      className="contents"
      onSubmit={(e) => {
        e.preventDefault()
        if (valid) onConfirm({ image: image.trim(), targetContainer: target.trim() })
      }}
    >
      <AlertDialogHeader className="place-items-start text-left">
        <AlertDialogTitle className="flex items-center gap-2 font-display text-base font-semibold tracking-wider text-accent-neon uppercase">
          <BugIcon aria-hidden="true" className="size-5 shrink-0" weight="bold" />
          Add debug container?
        </AlertDialogTitle>
        <AlertDialogDescription className="text-left text-xs text-muted-foreground">
          Adds an ephemeral container that shares the target container&apos;s process namespace.
        </AlertDialogDescription>
      </AlertDialogHeader>

      <p
        data-testid="debug-target-pod"
        className="border border-border bg-background px-2 py-1.5 font-mono text-sm break-all"
      >
        {item.namespace && <span className="text-muted-foreground">{item.namespace}/</span>}
        <span className="text-foreground">{item.name}</span>
        <span className="sr-only"> ({itemName(item)})</span>
      </p>

      <div className="flex flex-col gap-1.5">
        <label htmlFor={imageId} className="text-xs">
          Debug image
        </label>
        <Input
          id={imageId}
          data-testid="debug-image"
          required
          placeholder="busybox or nicolaka/netshoot"
          autoComplete="off"
          spellCheck={false}
          value={image}
          onChange={(e) => setImage(e.target.value)}
          className="h-[max(2rem,var(--touch-target))] font-mono"
        />
      </div>

      <div className="flex flex-col gap-1.5">
        <label htmlFor={targetId} className="text-xs">
          Target container
        </label>
        <Input
          id={targetId}
          data-testid="debug-target-container"
          required
          list={listId}
          autoComplete="off"
          spellCheck={false}
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          className="h-[max(2rem,var(--touch-target))] font-mono"
        />
        <datalist id={listId}>
          {containers.map((c) => (
            <option key={c} value={c} />
          ))}
        </datalist>
      </div>

      <p
        data-testid="debug-summary"
        aria-live="polite"
        className="border border-border bg-background px-2 py-1.5 font-mono text-xs break-all"
      >
        pod {itemName(item)} · target {target.trim() || '—'} · image {image.trim() || '—'}
      </p>

      <p
        data-testid="debug-warning"
        role="note"
        className="flex items-start gap-2 border border-destructive/60 px-2 py-1.5 text-xs text-destructive"
      >
        <WarningIcon aria-hidden="true" className="mt-0.5 size-4 shrink-0" weight="bold" />
        {EPHEMERAL_WARNING}
      </p>

      <AlertDialogFooter className="gap-2">
        <AlertDialogCancel
          data-testid="debug-cancel"
          className="h-[max(2rem,var(--touch-target))] min-w-24"
        >
          Cancel
        </AlertDialogCancel>
        <Button
          type="submit"
          variant="destructive"
          data-testid="debug-submit"
          disabled={!valid}
          className="h-[max(2rem,var(--touch-target))] min-w-24"
        >
          Add debug container
        </Button>
      </AlertDialogFooter>
    </form>
  )
}
