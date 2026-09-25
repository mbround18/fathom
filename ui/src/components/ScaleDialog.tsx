/**
 * Scale dialog (T095): a replica count ≥ 0 for one named object, shown as `current → new`
 * before anything is sent (US3 scenario 1). Esc and Cancel send nothing.
 *
 * Test hooks: `scale-dialog`, `scale-target`, `scale-replicas`, `scale-preview`,
 * `scale-cancel`, `scale-submit`.
 */
import { ArrowsOutLineVerticalIcon, MinusIcon, PlusIcon } from '@phosphor-icons/react'
import { useId, useState } from 'react'

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

import { itemName, MAX_REPLICAS, parseReplicas, type ConfirmItem } from './action-utils'

export interface ScaleDialogProps {
  open: boolean
  /** Kubernetes kind, e.g. `Deployment`. */
  kind: string
  item: ConfirmItem
  current: number
  onConfirm: (replicas: number) => void
  onCancel: () => void
  onCloseAutoFocus?: (e: Event) => void
}

export function ScaleDialog(props: ScaleDialogProps) {
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
          data-testid="scale-dialog"
          onCloseAutoFocus={onCloseAutoFocus}
          onOpenAutoFocus={(e) => {
            // Start in the count field (selected), not on Cancel: typing is the whole task.
            const input = (e.currentTarget as HTMLElement | null)?.querySelector('input')
            if (input) {
              e.preventDefault()
              input.focus()
              input.select()
            }
          }}
          className="flex w-[calc(100vw-2rem)] flex-col gap-3 data-[size=default]:max-w-md data-[size=default]:sm:max-w-md"
        >
          <ScaleBody {...props} />
        </AlertDialogContent>
      )}
    </AlertDialog>
  )
}

function ScaleBody({ kind, item, current, onConfirm }: ScaleDialogProps) {
  const [text, setText] = useState(String(current))
  const inputId = useId()
  const hintId = useId()
  const next = parseReplicas(text)
  const valid = next !== undefined
  const changed = valid && next !== current
  const step = (delta: number) => {
    const base = next ?? current
    setText(String(Math.max(0, Math.min(MAX_REPLICAS, base + delta))))
  }

  return (
    <form
      className="contents"
      onSubmit={(e) => {
        e.preventDefault()
        if (changed) onConfirm(next)
      }}
    >
      <AlertDialogHeader className="place-items-start text-left">
        <AlertDialogTitle className="flex items-center gap-2 font-display text-base font-semibold tracking-wider text-accent-neon uppercase">
          <ArrowsOutLineVerticalIcon aria-hidden="true" className="size-5 shrink-0" weight="bold" />
          Scale {kind.toLowerCase()}
        </AlertDialogTitle>
        <AlertDialogDescription className="text-left text-xs text-muted-foreground">
          Sets the desired replica count. Pods are added or removed by the controller.
        </AlertDialogDescription>
      </AlertDialogHeader>

      <p
        data-testid="scale-target"
        className="border border-border bg-background px-2 py-1.5 font-mono text-sm break-all"
      >
        {item.namespace && <span className="text-muted-foreground">{item.namespace}/</span>}
        <span className="text-foreground">{item.name}</span>
        <span className="sr-only"> ({itemName(item)})</span>
      </p>

      <div className="flex flex-col gap-1.5">
        <label htmlFor={inputId} className="text-xs">
          Replicas
        </label>
        <div className="flex items-stretch gap-0">
          <Button
            type="button"
            variant="outline"
            aria-label="Decrease replicas"
            data-testid="scale-decrement"
            disabled={(next ?? current) <= 0}
            onClick={() => step(-1)}
            className="size-[max(2rem,var(--touch-target))] border-r-0"
          >
            <MinusIcon aria-hidden="true" />
          </Button>
          <Input
            id={inputId}
            data-testid="scale-replicas"
            type="number"
            inputMode="numeric"
            min={0}
            max={MAX_REPLICAS}
            step={1}
            required
            aria-invalid={!valid}
            aria-describedby={hintId}
            value={text}
            onChange={(e) => setText(e.target.value)}
            className="h-[max(2rem,var(--touch-target))] flex-1 [appearance:textfield] text-center font-mono text-base tabular-nums [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
          />
          <Button
            type="button"
            variant="outline"
            aria-label="Increase replicas"
            data-testid="scale-increment"
            disabled={(next ?? current) >= MAX_REPLICAS}
            onClick={() => step(1)}
            className="size-[max(2rem,var(--touch-target))] border-l-0"
          >
            <PlusIcon aria-hidden="true" />
          </Button>
        </div>
        <p
          id={hintId}
          className={cn('text-xs', valid ? 'text-muted-foreground' : 'text-destructive')}
        >
          {valid ? (
            'A whole number, 0 or more.'
          ) : (
            <>
              <span aria-hidden="true">✖ </span>Enter a whole number from 0 to {MAX_REPLICAS}.
            </>
          )}
        </p>
      </div>

      <p
        data-testid="scale-preview"
        aria-live="polite"
        className="flex items-baseline justify-center gap-3 border border-border bg-background py-2 font-mono tabular-nums"
      >
        <span className="flex flex-col items-center">
          <span className="text-xs tracking-wider text-muted-foreground uppercase">current</span>
          <span data-testid="scale-current" className="text-lg">
            {current}
          </span>
        </span>
        <span aria-hidden="true" className="text-accent-neon">
          →
        </span>
        <span className="sr-only">to</span>
        <span className="flex flex-col items-center">
          <span className="text-xs tracking-wider text-muted-foreground uppercase">new</span>
          <span data-testid="scale-new" className="text-lg text-accent-neon">
            {valid ? next : '—'}
          </span>
        </span>
      </p>

      <AlertDialogFooter className="gap-2">
        <AlertDialogCancel
          data-testid="scale-cancel"
          className="h-[max(2rem,var(--touch-target))] min-w-24"
        >
          Cancel
        </AlertDialogCancel>
        <Button
          type="submit"
          data-testid="scale-submit"
          disabled={!changed}
          className="h-[max(2rem,var(--touch-target))] min-w-24"
        >
          {changed ? `Scale ${String(current)} → ${String(next)}` : 'Scale'}
        </Button>
      </AlertDialogFooter>
    </form>
  )
}
