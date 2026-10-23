import { beforeEach, describe, expect, it, vi } from 'vitest'

import { PROTOCOL_VERSION } from '@/protocol/generated'
import {
  backoffMs,
  Transport,
  wsUrl,
  type ConnectionStatus,
  type TerminalReason,
  type TransportEnv,
} from '@/protocol/transport'

import { FakeSocket } from './fake-socket'

/** Timers the test fires by hand, recording each requested delay. */
function manualTimers() {
  const queue: { fn: () => void; ms: number }[] = []
  return {
    delays: () => queue.map((t) => t.ms),
    fire: () => {
      const t = queue.shift()
      if (!t) throw new Error('no timer pending')
      t.fn()
    },
    pending: () => queue.length,
    setTimer: (fn: () => void, ms: number) => {
      queue.push({ fn, ms })
      return queue.length
    },
  }
}

function setup(origin = 'https://depths.example.com') {
  FakeSocket.reset()
  const timers = manualTimers()
  const env: TransportEnv = {
    origin,
    factory: (u) => new FakeSocket(u),
    setTimer: timers.setTimer,
    clearTimer: () => undefined,
  }
  const events: unknown[] = []
  const statuses: { status: ConnectionStatus; reconnected: boolean }[] = []
  const terminal: TerminalReason[] = []
  const t = new Transport(env, 'csrf-abc', {
    onEvent: (e) => events.push(e),
    onStatus: (status, { reconnected }) => statuses.push({ status, reconnected }),
    onTerminal: (r) => terminal.push(r),
  })
  return { t, timers, events, statuses, terminal }
}

describe('wsUrl', () => {
  it('uses wss on https and ws on http, at /api/v1/ws', () => {
    expect(wsUrl('https://depths.example.com')).toBe('wss://depths.example.com/api/v1/ws')
    expect(wsUrl('http://127.0.0.1:5174')).toBe('ws://127.0.0.1:5174/api/v1/ws')
  })
})

describe('Transport', () => {
  beforeEach(() => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
  })

  it('connects to the location-derived URL with no token in it', () => {
    setup()
    expect(FakeSocket.last().url).toBe('wss://depths.example.com/api/v1/ws')
    expect(FakeSocket.last().url).not.toContain('csrf')
  })

  it('sends Hello as the first frame, before commands queued while connecting', () => {
    const { t } = setup()
    const id = t.send({ type: 'ListStacks' })
    const s = FakeSocket.last()
    expect(s.sent).toHaveLength(0)
    s.open()
    expect(s.frames()).toEqual([
      { protocol_version: PROTOCOL_VERSION, csrf_token: 'csrf-abc' },
      { id, cmd: { type: 'ListStacks' } },
    ])
  })

  it('numbers commands and sends them straight away once open', () => {
    const { t } = setup()
    FakeSocket.last().open()
    const a = t.send({ type: 'ListStacks' })
    const b = t.send({ type: 'GetStack', data: { name: 'web' } })
    expect(b).toBe(a + 1)
    expect(FakeSocket.last().frames().slice(1)).toEqual([
      { id: a, cmd: { type: 'ListStacks' } },
      { id: b, cmd: { type: 'GetStack', data: { name: 'web' } } },
    ])
  })

  it('hands server envelopes to onEvent and ignores malformed frames', () => {
    const { events } = setup()
    const s = FakeSocket.last()
    s.open()
    s.serverSend({ reply_to: 1, event: { type: 'Invalid', data: { reason: 'x' } } })
    s.onmessage?.({ data: '{not json' })
    s.serverSend({ nope: true })
    expect(events).toEqual([{ reply_to: 1, event: { type: 'Invalid', data: { reason: 'x' } } }])
  })

  it('reconnects with doubling, capped backoff, and resets it after a successful open', () => {
    const { timers, statuses } = setup()
    FakeSocket.last().open()
    const seen: number[] = []
    for (let i = 0; i < 6; i++) {
      FakeSocket.last().serverClose(1006)
      seen.push(...timers.delays())
      timers.fire()
    }
    expect(seen).toEqual([500, 1000, 2000, 4000, 8000, 15_000])
    expect(backoffMs(20)).toBe(15_000)
    expect(FakeSocket.instances).toHaveLength(7)
    expect(statuses.filter((s) => s.status === 'reconnecting')).toHaveLength(6)

    // A successful open resets the attempt counter.
    FakeSocket.last().open()
    FakeSocket.last().serverClose(1006)
    expect(timers.delays()).toEqual([500])
  })

  it('sends Hello first again on every reconnect and reports reconnected', () => {
    const { t, timers, statuses } = setup()
    FakeSocket.last().open()
    FakeSocket.last().serverClose(1001)
    const queued = t.send({ type: 'ListStacks' })
    timers.fire()
    const s = FakeSocket.last()
    s.open()
    expect(s.frames()).toEqual([
      { protocol_version: PROTOCOL_VERSION, csrf_token: 'csrf-abc' },
      { id: queued, cmd: { type: 'ListStacks' } },
    ])
    expect(statuses.at(-1)).toEqual({ status: 'open', reconnected: true })
  })

  it.each([
    [4401, 'unauthenticated'],
    [4403, 'forbidden'],
    [4429, 'too_many'],
  ] as const)('treats close %i as terminal (%s): no reconnect', (code, reason) => {
    const { t, timers, terminal } = setup()
    FakeSocket.last().open()
    FakeSocket.last().serverClose(code)
    expect(terminal).toEqual([reason])
    expect(timers.pending()).toBe(0)
    t.send({ type: 'ListStacks' })
    expect(FakeSocket.instances).toHaveLength(1)
  })

  it('close() stops for good', () => {
    const { t, timers } = setup()
    FakeSocket.last().open()
    t.close()
    expect(timers.pending()).toBe(0)
    expect(FakeSocket.instances).toHaveLength(1)
  })
})
