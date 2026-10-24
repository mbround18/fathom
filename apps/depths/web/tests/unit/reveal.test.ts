import { renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { Cmd, Event, EventEnvelope, Me } from '@/protocol/generated'

const bus = vi.hoisted(() => ({
  sent: [] as Cmd[],
  listeners: [] as ((e: EventEnvelope) => void)[],
  reconnect: [] as (() => void)[],
}))
vi.mock('@/app/connection', () => ({
  sendCmd: (cmd: Cmd) => {
    bus.sent.push(cmd)
    return bus.sent.length
  },
  onServerEvent: (l: (e: EventEnvelope) => void) => {
    bus.listeners.push(l)
    return () => undefined
  },
  onReconnected: (l: () => void) => {
    bus.reconnect.push(l)
    return () => undefined
  },
  connect: () => undefined,
  disconnect: () => undefined,
}))

const { useReveal, useCanReveal, resetRevealStore, REVEAL_TTL_MS } = await import('@/features/editor/reveal-store')
const { useSession } = await import('@/app/session-store')

function emit(event: Event, reply_to?: number): void {
  for (const l of bus.listeners) l(reply_to === undefined ? { event } : { reply_to, event })
}

function me(role: Me['role']): Me {
  return {
    identity: { provider: 'github', subject: '1', username: 'ada', email: null },
    csrf_token: 'c',
    role,
    owner: false,
    stacks: [],
    features: { shell: true, edit: true },
  }
}

beforeEach(() => {
  vi.useFakeTimers()
  bus.sent.length = 0
  resetRevealStore()
})
afterEach(() => vi.useRealTimers())

describe('reveal store', () => {
  it('masks revealed text again after REVEAL_TTL_MS', () => {
    useReveal.getState().reveal('web')
    expect(bus.sent).toEqual([{ type: 'RevealEnv', data: { name: 'web' } }])
    emit({ type: 'EnvRevealed', data: { name: 'web', env_text: 'DB_PASSWORD=hunter2\n' } }, 1)
    expect(useReveal.getState().env.web).toEqual({ state: 'ready', text: 'DB_PASSWORD=hunter2\n' })

    vi.advanceTimersByTime(REVEAL_TTL_MS - 1)
    expect(useReveal.getState().env.web?.state).toBe('ready')
    vi.advanceTimersByTime(1)
    expect(useReveal.getState().env.web).toBeUndefined()
  })

  it('forgetting clears the timer and a reply still pending', () => {
    useReveal.getState().reveal('web')
    useReveal.getState().forget('web')
    emit({ type: 'EnvRevealed', data: { name: 'web', env_text: 'X_TOKEN=t\n' } }, 1)
    expect(useReveal.getState().env.web).toBeUndefined()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('only an admin may reveal', () => {
    useSession.setState({ status: 'ready', me: me('user') })
    expect(renderHook(() => useCanReveal()).result.current).toBe(false)
    useSession.setState({ status: 'ready', me: me('admin') })
    expect(renderHook(() => useCanReveal()).result.current).toBe(true)
    useSession.setState({ status: 'signed_out', me: null })
    expect(renderHook(() => useCanReveal()).result.current).toBe(false)
  })
})
