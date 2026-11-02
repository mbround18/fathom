import { useState } from 'react'

import { Button } from '../../src/button'
import { ConfirmDialog, type Preview } from '../../src/kit'
import type { Fixture } from '../fixture'

const PREVIEW: Preview = {
  summary: 'Delete 12 pods in fathom-dev.',
  affected: ['pod/api-0', 'pod/api-1', 'pod/worker-0'],
  step_up: false,
}

function Typed({ startOpen = true }: { startOpen?: boolean }) {
  const [preview, setPreview] = useState<Preview | null>(startOpen ? PREVIEW : null)
  const [confirmed, setConfirmed] = useState(0)
  return (
    <div className="flex flex-col items-start gap-2">
      <Button
        variant="outline"
        onClick={() => {
          setPreview(PREVIEW)
        }}
      >
        Delete 12 pods
      </Button>
      <p className="text-xs text-muted-foreground">
        Confirmed: <span data-confirmed>{confirmed}</span>
      </p>
      <ConfirmDialog
        preview={preview}
        title="Delete 12 pods?"
        confirmLabel="Delete"
        tone="destructive"
        typedConfirm={{ word: '12' }}
        onCancel={() => {
          setPreview(null)
        }}
        onConfirm={() => {
          setConfirmed((n) => n + 1)
          setPreview(null)
        }}
      />
    </div>
  )
}

const fixture: Fixture = {
  id: 'confirm-dialog-typed',
  states: {
    open: () => <Typed />,
    closed: () => <Typed startOpen={false} />,
  },
}

export default fixture
