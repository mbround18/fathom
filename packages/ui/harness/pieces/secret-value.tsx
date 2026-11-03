import { useState } from 'react'

import { Field, FORM_WIDTH, SecretInput, SecretValue } from '../../src/kit'
import type { Fixture } from '../fixture'

const SECRET = 'wsk_live_4f8a2c9e7b1d3f5a6c8e0b2d4f6a8c0e'

/** Stands in for the caller's step-up: asks first, then hands the value over or backs out. */
function StepUp() {
  const [asking, setAsking] = useState<((ok: boolean) => void) | null>(null)
  return (
    <div className="flex max-w-xl flex-col gap-3">
      <SecretValue
        label="Widget signing key"
        data="key"
        hideAfterMs={3000}
        reveal={() =>
          new Promise<string | null>((resolve) => {
            setAsking(() => (ok: boolean) => {
              setAsking(null)
              resolve(ok ? SECRET : null)
            })
          })
        }
      />
      {asking && (
        <div role="group" aria-label="Confirm it is you" className="flex gap-2 text-xs">
          <button type="button" className="border border-border px-2 py-1" onClick={() => asking(true)}>
            Confirm
          </button>
          <button type="button" className="border border-border px-2 py-1" onClick={() => asking(false)}>
            Cancel
          </button>
        </div>
      )}
    </div>
  )
}

const fixture: Fixture = {
  id: 'secret-value',
  states: {
    reveal: () => <StepUp />,
    failed: () => (
      <div className="max-w-xl">
        <SecretValue label="Widget token" data="token" reveal={() => Promise.reject(new Error('Step-up was refused.'))} />
      </div>
    ),
    input: () => (
      <div className={FORM_WIDTH}>
        <Field label="New value" htmlFor="secret-new">
          <SecretInput id="secret-new" name="value" />
        </Field>
      </div>
    ),
  },
}

export default fixture
