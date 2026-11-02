import { useState } from 'react'

import { LoadFailed, type Refusal } from '../../src/kit'
import type { Fixture } from '../fixture'

const REFUSAL: Refusal = { kind: 'error', code: 'unavailable', message: 'The widget store did not answer in time.' }

/** Retry counts its presses, so the spec can see the callback run. */
function WithRetry() {
  const [tries, setTries] = useState(0)
  return (
    <div className="flex flex-col gap-2">
      <LoadFailed
        what="widgets"
        refusal={REFUSAL}
        onRetry={() => {
          setTries((n) => n + 1)
        }}
      />
      <p data-retries className="text-xs text-muted-foreground">
        Retried {tries} {tries === 1 ? 'time' : 'times'}
      </p>
    </div>
  )
}

const fixture: Fixture = {
  id: 'load-failed',
  states: {
    retry: () => <WithRetry />,
    'no-retry': () => <LoadFailed what="widgets" refusal={{ kind: 'denied', action: 'list_widgets', reason: 'no_grant' }} />,
  },
}

export default fixture
