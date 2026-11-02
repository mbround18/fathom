import { useState } from 'react'

import { LoadState, type Refusal } from '../../src/kit'
import type { Fixture } from '../fixture'

const REFUSAL: Refusal = { kind: 'error', code: 'unavailable', message: 'The widget store did not answer in time.' }

function Widgets() {
  return (
    <ul aria-label="Widgets" className="flex flex-col gap-1 text-sm">
      <li>Blue widget</li>
      <li>Green widget</li>
    </ul>
  )
}

/** Fails until Retry, then loads: the failed state hands over to the children. */
function FailedThenLoaded() {
  const [failed, setFailed] = useState<Refusal | null>(REFUSAL)
  return (
    <LoadState
      loading={false}
      failed={failed}
      what="widgets"
      onRetry={() => {
        setFailed(null)
      }}
    >
      <Widgets />
    </LoadState>
  )
}

const fixture: Fixture = {
  id: 'load-state',
  states: {
    loading: () => (
      <LoadState loading label="Loading widgets…">
        <Widgets />
      </LoadState>
    ),
    failed: () => <FailedThenLoaded />,
    loaded: () => (
      <LoadState loading={false} failed={null}>
        <Widgets />
      </LoadState>
    ),
  },
}

export default fixture
