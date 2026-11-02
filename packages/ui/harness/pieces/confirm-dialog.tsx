import { useState } from 'react'

import { Button } from '../../src/button'
import { ConfirmDialog, RefusalText, type Preview, type Refusal } from '../../src/kit'
import type { Fixture } from '../fixture'

const PREVIEW: Preview = { summary: 'Remove Blue widget.', affected: [], step_up: false }
const AFFECTED: Preview = {
  summary: 'Remove the widget group “Primary”.',
  affected: ['widget/blue', 'widget/green', 'widget/red-with-a-much-longer-name-that-has-to-wrap-on-a-phone'],
  step_up: false,
}
const STEP_UP: Preview = { summary: 'Rotate the widget signing key.', affected: [], step_up: true }
const DENIED: Refusal = { kind: 'denied', action: 'remove_widget', reason: 'no_grant' }

/**
 * Opens the preview from a trigger. On Confirm the change is denied (the refusal shows in the
 * dialog, which stays open) or done (the dialog closes and the page says so).
 */
function Flow({ preview, outcome = 'done' }: { preview: Preview; outcome?: 'done' | 'denied' }) {
  const [open, setOpen] = useState(false)
  const [refusal, setRefusal] = useState<Refusal | null>(null)
  const [done, setDone] = useState<string | null>(null)
  const [cancelled, setCancelled] = useState(0)
  return (
    <div className="flex flex-col items-start gap-2">
      <Button
        type="button"
        variant="outline"
        onClick={() => {
          setRefusal(null)
          setOpen(true)
        }}
      >
        Remove widget
      </Button>
      <p role="status" className="text-sm text-muted-foreground">
        {done ?? (cancelled > 0 ? 'Nothing changed.' : '')}
      </p>
      <ConfirmDialog
        preview={open ? preview : null}
        title="Remove widget"
        confirmLabel="Remove"
        tone="destructive"
        data-confirm-fixture
        onCancel={() => {
          setOpen(false)
          setCancelled((n) => n + 1)
        }}
        onConfirm={() => {
          if (outcome === 'denied') {
            setRefusal(DENIED)
            return
          }
          setOpen(false)
          setDone('Widget removed.')
        }}
      >
        {refusal && <RefusalText refusal={refusal} />}
      </ConfirmDialog>
    </div>
  )
}

const fixture: Fixture = {
  id: 'confirm-dialog',
  states: {
    preview: () => <Flow preview={PREVIEW} />,
    affected: () => <Flow preview={AFFECTED} />,
    'step-up': () => <Flow preview={STEP_UP} />,
    denied: () => <Flow preview={PREVIEW} outcome="denied" />,
    done: () => <Flow preview={PREVIEW} outcome="done" />,
  },
}

export default fixture
