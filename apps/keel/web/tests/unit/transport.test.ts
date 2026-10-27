import { beforeEach, describe, expect, it, vi } from 'vitest'

import {
  backoffMs,
  CLIENT_NAME,
  CloseCode,
  Transport,
  wsUrl,
  type ConnectionStatus,
  type TerminalReason,
  type TransportEnv,
} from '@/protocol/transport'
import { PROTOCOL_VERSION } from '@/protocol/wire'

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

function setup(origin = 'https://keel.example.com') {
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

const HELLO = { protocol_version: PROTOCOL_VERSION, client: CLIENT_NAME, csrf_token: 'csrf-abc' }

describe('wsUrl', () => {
  it('uses wss on https and ws on http, at /api/v1/ws', () => {
    expect(wsUrl('https://keel.example.com')).toBe('wss://keel.example.com/api/v1/ws')
    expect(wsUrl('http://127.0.0.1:5175')).toBe('ws://127.0.0.1:5175/api/v1/ws')
  })
})

describe('Transport', () => {
  beforeEach(() => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
  })

  it('connects to the location-derived URL with no token in it', () => {
    setup()
    expect(FakeSocket.last().url).toBe('wss://keel.example.com/api/v1/ws')
    expect(FakeSocket.last().url).not.toContain('csrf')
  })

  it('names itself in Hello, keel-web/<version>, within the 64-byte limit', () => {
    expect(CLIENT_NAME).toMatch(/^keel-web\/\d+\.\d+\.\d+/)
    expect(CLIENT_NAME.length).toBeLessThanOrEqual(64)
  })

  it('sends Hello as the first frame, before commands queued while connecting', () => {
    const { t } = setup()
    const id = t.send({ type: 'GetMe' })
    const s = FakeSocket.last()
    expect(s.sent).toHaveLength(0)
    s.open()
    expect(s.frames()).toEqual([HELLO, { id, cmd: { type: 'GetMe' } }])
  })

  it('numbers commands and sends them straight away once open', () => {
    const { t } = setup()
    FakeSocket.last().open()
    const a = t.send({ type: 'GetMe' })
    const b = t.send({ type: 'PasskeyBegin', data: { purpose: { kind: 'verify' } } })
    expect(b).toBe(a + 1)
    expect(FakeSocket.last().frames().slice(1)).toEqual([
      { id: a, cmd: { type: 'GetMe' } },
      { id: b, cmd: { type: 'PasskeyBegin', data: { purpose: { kind: 'verify' } } } },
    ])
  })

  it('hands server envelopes to onEvent and ignores malformed frames', () => {
    const { events } = setup()
    const s = FakeSocket.last()
    s.open()
    s.serverSend({ reply_to: 1, event: { type: 'OpError', data: { code: 'x', message: 'y' } } })
    s.onmessage?.({ data: '{not json' })
    s.serverSend({ nope: true })
    expect(events).toEqual([{ reply_to: 1, event: { type: 'OpError', data: { code: 'x', message: 'y' } } }])
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

    FakeSocket.last().open()
    FakeSocket.last().serverClose(1006)
    expect(timers.delays()).toEqual([500])
  })

  it('sends Hello first again on every reconnect and reports reconnected', () => {
    const { t, timers, statuses } = setup()
    FakeSocket.last().open()
    FakeSocket.last().serverClose(1001)
    const queued = t.send({ type: 'GetMe' })
    timers.fire()
    const s = FakeSocket.last()
    s.open()
    expect(s.frames()).toEqual([HELLO, { id: queued, cmd: { type: 'GetMe' } }])
    expect(statuses.at(-1)).toEqual({ status: 'open', reconnected: true })
  })

  it.each([
    [CloseCode.SignedOut, 'unauthenticated'],
    [CloseCode.Csrf, 'unauthenticated'],
    [CloseCode.Protocol, 'outdated'],
  ] as const)('treats close %i as terminal (%s): no reconnect', (code, reason) => {
    const { t, timers, terminal } = setup()
    FakeSocket.last().open()
    FakeSocket.last().serverClose(code)
    expect(terminal).toEqual([reason])
    expect(timers.pending()).toBe(0)
    t.send({ type: 'GetMe' })
    expect(FakeSocket.instances).toHaveLength(1)
  })

  it('Revoked ends the transport at once, before the close that follows it', () => {
    const { events, timers, terminal } = setup()
    const s = FakeSocket.last()
    s.open()
    s.serverSend({ event: { type: 'Revoked' } })
    expect(terminal).toEqual(['revoked'])
    expect(events).toEqual([{ event: { type: 'Revoked' } }])
    // The server's 4401 that follows is not reported a second time, and nothing reconnects.
    s.serverClose(CloseCode.SignedOut)
    expect(terminal).toEqual(['revoked'])
    expect(timers.pending()).toBe(0)
    expect(FakeSocket.instances).toHaveLength(1)
  })

  it('close() stops for good', () => {
    const { t, timers, statuses } = setup()
    FakeSocket.last().open()
    t.close()
    expect(timers.pending()).toBe(0)
    expect(FakeSocket.instances).toHaveLength(1)
    expect(statuses.at(-1)).toEqual({ status: 'closed', reconnected: false })
  })
})
