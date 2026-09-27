/**
 * #40: Reveal fetches one key on demand (`RevealSecretValue`) and the answer goes only to the
 * caller: it is never written into the store.
 */
import { beforeEach, describe, expect, it } from 'vitest'

import type { BackendCmd, SecretRevealOutcome } from '@/protocol/types'
import { initialData, useStore } from '@/store'
import { resetActions, setActionTimeout } from '@/store/actions'
import { ingest } from '@/store/ingest'
import { resetSecretReveals, revealSecretValue } from '@/store/secretReveal'

import { twoContexts } from '../fixtures'
import { batch, envelope, globalRoute, nsRoute, read } from '../fixtures/builders'

const VALUE = 'hunter2-super-secret'
let sent: BackendCmd[]

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
  resetSecretReveals()
  setActionTimeout()
  ingest(twoContexts)
  sent = []
  useStore.setState({
    request: (cmd) => {
      sent.push(cmd)
      return Promise.resolve(41)
    },
  })
})

function answer(outcome: SecretRevealOutcome, ok = true, error: string | null = null) {
  const req = { type: 'Request' as const, data: 41 }
  ingest(
    batch([
      read(
        20,
        nsRoute('kind-a', 'config/secrets', 'ns'),
        {
          type: 'SecretValueRevealed',
          data: { namespace: 'ns', name: 'db', key: 'password', outcome },
        },
        req,
      ),
      envelope(21, globalRoute(), { type: 'CommandFinished', data: { ok, error } }, req),
    ]),
  )
}

describe('revealSecretValue', () => {
  it('asks for exactly one key and returns the value without storing it', async () => {
    const pending = revealSecretValue('ns', 'db', 'password')
    await Promise.resolve()
    expect(sent).toEqual([
      { type: 'RevealSecretValue', data: { namespace: 'ns', name: 'db', key: 'password' } },
    ])
    answer({ type: 'Value', data: { value: VALUE } })
    await expect(pending).resolves.toEqual({ type: 'Value', data: { value: VALUE } })
    expect(JSON.stringify(useStore.getState())).not.toContain(VALUE)
  })

  it('passes a typed forbidden answer through', async () => {
    const pending = revealSecretValue('ns', 'db', 'password')
    await Promise.resolve()
    answer({ type: 'Forbidden', data: { message: 'secrets "db" is forbidden' } })
    await expect(pending).resolves.toEqual({
      type: 'Forbidden',
      data: { message: 'secrets "db" is forbidden' },
    })
  })

  it('reports a failed command without an answer', async () => {
    const pending = revealSecretValue('ns', 'db', 'password')
    await Promise.resolve()
    ingest(
      batch([
        envelope(
          30,
          globalRoute(),
          { type: 'CommandFinished', data: { ok: false, error: 'No cluster connection' } },
          { type: 'Request', data: 41 },
        ),
      ]),
    )
    await expect(pending).resolves.toEqual({
      type: 'Failed',
      data: { message: 'No cluster connection' },
    })
  })

  it('reports a request that could not be sent', async () => {
    useStore.setState({ request: () => Promise.reject(new Error('Not connected')) })
    await expect(revealSecretValue('ns', 'db', 'password')).resolves.toEqual({
      type: 'NoAnswer',
      data: { message: 'Not connected' },
    })
  })
})
