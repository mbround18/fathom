import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { descriptorFor } from '@/resources'
import { parseQuantity } from '@/resources/batchCCommon'
import { pdbStatus } from '@/resources/configKinds'
import { SecretKeys, type RevealFn } from '@/resources/secretKeys'

const SECRET_VALUE = 'hunter2-super-secret'

type Outcome = Awaited<ReturnType<RevealFn>>
const valueOf = (value: string): Outcome => ({ type: 'Value', data: { value } })

/** A reveal function answering from a table, recording which keys were asked for. */
function fakeReveal(answers: Record<string, Outcome>) {
  const asked: string[] = []
  const fn: RevealFn = (key) => {
    asked.push(key)
    const answer = answers[key]
    return answer ? Promise.resolve(answer) : new Promise(() => undefined)
  }
  return { fn, asked }
}

function renderKeys(keys: string[], reveal: RevealFn) {
  return render(<SecretKeys keys={keys} namespace="ns" name="db" reveal={reveal} />)
}

describe('SecretKeys', () => {
  it('shows no values until reveal, fetches only the revealed key, and re-masks', async () => {
    const { fn, asked } = fakeReveal({ password: valueOf(SECRET_VALUE), user: valueOf('admin') })
    const { container } = renderKeys(['password', 'user'], fn)
    expect(screen.getByText('password')).toBeTruthy()
    expect(container.innerHTML).not.toContain(SECRET_VALUE)
    expect(asked).toEqual([])
    expect(screen.queryByLabelText('Copy value of password')).toBeNull()

    fireEvent.click(screen.getByLabelText('Reveal value of password'))
    expect(asked).toEqual(['password'])
    await screen.findByText(SECRET_VALUE)
    expect(container.innerHTML).not.toContain('admin')
    expect(screen.getByLabelText('Copy value of password')).toBeTruthy()

    fireEvent.keyDown(screen.getByTestId('secret-keys'), { key: 'Escape' })
    expect(container.innerHTML).not.toContain(SECRET_VALUE)
    expect(screen.queryByLabelText('Copy value of password')).toBeNull()
  })

  it('re-masks on blur out of the section and on unmount', async () => {
    const { fn } = fakeReveal({ password: valueOf(SECRET_VALUE) })
    const { container, unmount } = renderKeys(['password'], fn)
    fireEvent.click(screen.getByLabelText('Reveal value of password'))
    await screen.findByText(SECRET_VALUE)
    fireEvent.blur(screen.getByTestId('secret-keys'), { relatedTarget: document.body })
    expect(container.innerHTML).not.toContain(SECRET_VALUE)
    fireEvent.click(screen.getByLabelText('Reveal value of password'))
    await screen.findByText(SECRET_VALUE)
    unmount()
    expect(document.body.innerHTML).not.toContain(SECRET_VALUE)
  })

  it('shows a loading state, and drops an answer that arrives after Hide', async () => {
    let answer: (o: Outcome) => void = () => undefined
    const reveal: RevealFn = () =>
      new Promise((resolve) => {
        answer = resolve
      })
    const { container } = renderKeys(['token'], reveal)
    fireEvent.click(screen.getByLabelText('Reveal value of token'))
    expect(screen.getByTestId('secret-value-token').textContent).toBe('Revealing…')
    fireEvent.click(screen.getByLabelText('Hide value of token'))
    await act(async () => {
      answer(valueOf(SECRET_VALUE))
      await Promise.resolve()
    })
    expect(container.innerHTML).not.toContain(SECRET_VALUE)
  })

  it.each<[string, Outcome, RegExp]>([
    [
      'forbidden',
      { type: 'Forbidden', data: { message: 'secrets "db" is forbidden' } },
      /^Forbidden: .*secrets "db" is forbidden/,
    ],
    ['not found', { type: 'NotFound' }, /^Not found: this Secret no longer exists/],
    ['a missing key', { type: 'KeyMissing' }, /no longer in the Secret/],
    ['binary', { type: 'Binary', data: { size: 42 } }, /Binary value \(42 bytes\)/],
    ['empty', valueOf(''), /^\(empty value\)$/],
    ['a failure', { type: 'Failed', data: { message: 'boom' } }, /Could not read the value: boom/],
  ])('says so plainly for %s, and offers Copy only for text values', async (_, outcome, text) => {
    const { fn } = fakeReveal({ token: outcome })
    renderKeys(['token'], fn)
    fireEvent.click(screen.getByLabelText('Reveal value of token'))
    await waitFor(() => {
      expect(screen.getByTestId('secret-value-token').textContent).toMatch(text)
    })
    expect(screen.queryByText(/not transferred/)).toBeNull()
    const copy = screen.queryByLabelText('Copy value of token')
    if (outcome.type === 'Value') expect(copy).toBeTruthy()
    else expect(copy).toBeNull()
  })
})

describe('batch C descriptors', () => {
  it('registers all eight sections with delete gated and confirm-prompted', () => {
    for (const s of [
      'storage/persistentvolumes',
      'storage/persistentvolumeclaims',
      'storage/storageclasses',
      'config/configmaps',
      'config/secrets',
      'config/resourcequotas',
      'config/limitranges',
      'config/poddisruptionbudgets',
    ]) {
      const d = descriptorFor(s)
      expect(d, s).toBeDefined()
      const del = d?.actions.find((a) => a.id === 'delete')
      expect(del?.prompt).toBe('confirm')
      expect(del && 'accessCheck' in del).toBe(true)
    }
  })

  it('cluster-scoped deletes send name only', () => {
    const d = descriptorFor('storage/storageclasses')
    const row = {
      name: 'standard',
      provisioner: 'p',
      reclaim_policy: null,
      volume_binding_mode: null,
      is_default: false,
    }
    expect(d?.actions[0]?.plan([row as never], {})[0]?.cmd).toEqual({
      type: 'DeleteStorageClass',
      data: { name: 'standard' },
    })
  })

  it('secret detail keys section shows names only', () => {
    const d = descriptorFor('config/secrets')
    const detail = {
      namespace: 'default',
      name: 's',
      creation_timestamp: '2024-01-01T00:00:00Z',
      labels: {},
      annotations: {},
      secret_type: 'Opaque',
      keys: ['password'],
      raw_json: '{}',
    }
    const block = d?.detailSections.find((s) => s.id === 'keys')?.render({ detail })
    expect(block?.type).toBe('list')
    const { container } = render(<>{block?.type === 'list' ? block.items : null}</>)
    expect(container.innerHTML).toContain('password')
    expect(screen.getByTestId('secret-value-password').textContent).toBe('••••••••')
  })

  it('helpers', () => {
    expect(parseQuantity('500m')).toBe(0.5)
    expect(parseQuantity('1Gi')).toBe(1024 ** 3)
    expect(pdbStatus({ current_healthy: 1, desired_healthy: 2 })).toBe('Disrupted')
    expect(pdbStatus({ current_healthy: null, desired_healthy: 2 })).toBe('Unknown')
  })
})
