/** Capability gate (033 US5): fail closed, verbatim reasons, stale-answer discard, invalidation. */
import { act, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import {
  Gated,
  ingestCapabilityChecked,
  ingestCapabilitySet,
  resetCapabilities,
  stateFor,
  toQuery,
  useCapabilityStore,
} from '@/lib/capabilities'
import type { BackendCmd, CapabilityResult } from '@/protocol/types'
import { useStore } from '@/store'

const sent: BackendCmd[] = []
const target = { verb: 'delete', resource: 'pods', namespace: 'dev' }

function answer(result: Partial<CapabilityResult>, context = 'ctx-a') {
  act(() => {
    ingestCapabilityChecked(toQuery(context, target), {
      allowed: false,
      reason: null,
      source: 'Review',
      verified: true,
      ...result,
    })
  })
}

function setContext(activeContext: string | undefined) {
  useStore.setState((s) => ({ session: { ...s.session, activeContext } }))
}

beforeEach(() => {
  sent.length = 0
  resetCapabilities()
  useStore.setState({ send: (cmd: BackendCmd) => void sent.push(cmd) })
  setContext('ctx-a')
})

afterEach(() => {
  resetCapabilities()
  vi.useRealTimers()
})

function mount() {
  render(
    <Gated {...target}>
      <button data-testid="del">Delete</button>
    </Gated>,
  )
  return screen.getByTestId('del')
}

describe('<Gated>', () => {
  it('is disabled while pending and asks the backend once (resolve, then check)', () => {
    const btn = mount()
    expect(btn).toBeDisabled()
    expect(btn).toHaveAttribute('data-capability', 'pending')
    expect(sent.map((c) => c.type)).toEqual(['ResolveCapabilities', 'CheckCapability'])
  })

  it('enables on a verified allow', () => {
    const btn = mount()
    answer({ allowed: true })
    expect(btn).toBeEnabled()
  })

  it('disables a denial and shows the API server reason verbatim', () => {
    const btn = mount()
    answer({ reason: 'pods "x" is forbidden: User "bob" cannot delete' })
    expect(btn).toBeDisabled()
    expect(btn).toHaveAttribute('title', 'pods "x" is forbidden: User "bob" cannot delete')
  })

  it('fails closed with "could not verify" when the check could not complete', () => {
    const btn = mount()
    answer({ verified: false, allowed: true, reason: 'connection refused' })
    expect(btn).toBeDisabled()
    expect(btn.getAttribute('title')).toMatch(/could not verify/i)
    expect(stateFor(undefined).status).toBe('pending')
  })

  it('discards an answer for a context that is no longer active', () => {
    const btn = mount()
    answer({ allowed: true }, 'ctx-old')
    expect(btn).toBeDisabled()
    expect(useCapabilityStore.getState().results).toEqual({})
    act(() => {
      ingestCapabilitySet({
        context: 'ctx-old',
        namespace: 'dev',
        identity_sub: '',
        rules: [],
        incomplete: false,
        fetched_at: 0,
      })
    })
    expect(useCapabilityStore.getState().sets).toEqual({})
  })

  it('re-asks after invalidation (identity/context change, RBAC apply)', () => {
    const btn = mount()
    answer({ allowed: true })
    expect(btn).toBeEnabled()
    sent.length = 0
    act(() => {
      resetCapabilities()
    })
    expect(btn).toBeDisabled()
    expect(sent.map((c) => c.type)).toContain('CheckCapability')
  })

  it('marks answers stale after the 60 s TTL without polling', () => {
    vi.useFakeTimers()
    const btn = mount()
    answer({ allowed: true })
    sent.length = 0
    act(() => {
      vi.advanceTimersByTime(59_000)
    })
    expect(sent).toEqual([])
    act(() => {
      vi.advanceTimersByTime(2_000)
    })
    expect(sent.map((c) => c.type)).toContain('CheckCapability')
    expect(btn).toBeEnabled() // stale answers keep showing until the fresh one lands
  })

  it('is disabled with no connected context', () => {
    setContext(undefined)
    const btn = mount()
    expect(btn).toBeDisabled()
    expect(sent).toEqual([])
  })
})
