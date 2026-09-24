import { describe, expect, it, vi } from 'vitest'

import {
  backoffMs,
  createHostedTransport,
  HostedTransport,
  hostedWsUrl,
  signInUrl,
  type HostedEnv,
  type HostedSessionInfo,
} from '@/protocol/hosted-transport'
import type { WebSocketLike } from '@/protocol/ws-transport'

class Sock implements WebSocketLike {
  readyState = 0
  onopen: ((ev: Event) => void) | null = null
  onmessage: ((ev: MessageEvent) => void) | null = null
  onclose: ((ev: CloseEvent) => void) | null = null
  onerror: ((ev: Event) => void) | null = null
  frames: unknown[] = []
  readonly url: string
  constructor(url: string) {
    this.url = url
  }
  send(d: string) {
    this.frames.push(JSON.parse(d))
  }
  close() {
    this.readyState = 3
  }
  open() {
    this.readyState = 1
    this.onopen?.(new Event('open'))
  }
  reply(m: unknown) {
    this.onmessage?.({ data: JSON.stringify(m) } as MessageEvent)
  }
  drop(code: number) {
    this.readyState = 3
    this.onclose?.({ code } as CloseEvent)
  }
}

function sock(socks: Sock[], i: number): Sock {
  const s = socks[i]
  if (!s) throw new Error(`no socket ${String(i)}`)
  return s
}

const session: HostedSessionInfo = {
  username: 'alice',
  groups: [],
  csrf: 'tok',
  expiresAt: '',
  idleTimeoutSec: 60,
  accessModel: 'x',
  mode: 'hosted',
}

function env(over: Partial<HostedEnv> = {}) {
  const socks: Sock[] = []
  const timers: (() => void)[] = []
  const e: HostedEnv = {
    origin: 'https://fathom.example',
    pathname: '/pods',
    search: '?ns=a',
    hash: '',
    fetch: vi.fn(),
    factory: (u) => {
      const s = new Sock(u)
      socks.push(s)
      return s
    },
    redirect: vi.fn(),
    setTimer: (fn) => timers.push(fn),
    clearTimer: () => undefined,
    ...over,
  }
  return { e, socks, timers }
}

describe('hosted transport', () => {
  it('uses same-origin wss with no token in the URL', () => {
    expect(hostedWsUrl('https://h.example')).toBe('wss://h.example/ws')
    expect(hostedWsUrl('http://localhost:3000')).toBe('ws://localhost:3000/ws')
    const { e, socks } = env()
    new HostedTransport(e, session)
    expect(sock(socks, 0).url).not.toMatch(/token/)
  })

  it('preserves the return path and rejects non-local login URLs', () => {
    expect(signInUrl('/auth/login', '/pods', '?ns=a')).toBe('/auth/login?return=%2Fpods%3Fns%3Da')
    expect(signInUrl('//evil.example', '/', '')).toMatch(/^\/auth\/login/)
  })

  it('bootstrap 401 redirects to sign-in', async () => {
    const { e } = env({
      fetch: vi.fn().mockResolvedValue({
        status: 401,
        ok: false,
        json: () => Promise.resolve({ error: 'unauthenticated', loginUrl: '/auth/login' }),
      }) as unknown as typeof fetch,
    })
    void createHostedTransport(e)
    await vi.waitFor(() => {
      expect(e.redirect).toHaveBeenCalledWith('/auth/login?return=%2Fpods%3Fns%3Da')
    })
  })

  it('bootstrap success yields a transport with the csrf token', async () => {
    const { e } = env({
      fetch: vi
        .fn()
        .mockResolvedValue({ status: 200, ok: true, json: () => Promise.resolve(session) }),
    })
    const t = (await createHostedTransport(e)) as HostedTransport
    expect(t.csrf).toBe('tok')
  })

  it('answers send in FIFO order and queues before open', async () => {
    const { e, socks } = env()
    const t = new HostedTransport(e, session)
    const p = t.send({ type: 'Ping' } as never)
    sock(socks, 0).open()
    expect(sock(socks, 0).frames).toHaveLength(1)
    sock(socks, 0).reply({ op: 'sent', request_id: 7 })
    await expect(p).resolves.toBe(7)
  })

  it('4401 redirects to sign-in and goes gone', () => {
    const { e, socks } = env()
    const t = new HostedTransport(e, session)
    const gone = vi.fn()
    t.onGone(gone)
    sock(socks, 0).open()
    sock(socks, 0).drop(4401)
    expect(e.redirect).toHaveBeenCalledWith('/auth/login?return=%2Fpods%3Fns%3Da')
    expect(gone).toHaveBeenCalled()
  })

  it('4429 is terminal without redirect', () => {
    const { e, socks } = env()
    const t = new HostedTransport(e, session)
    const gone = vi.fn()
    t.onGone(gone)
    sock(socks, 0).drop(4429)
    expect(gone).toHaveBeenCalled()
    expect(e.redirect).not.toHaveBeenCalled()
  })

  it('reconnects with backoff and re-subscribes', () => {
    const { e, socks, timers } = env()
    const t = new HostedTransport(e, session)
    t.subscribe(() => undefined).catch(() => undefined)
    sock(socks, 0).open()
    sock(socks, 0).drop(1006)
    expect(socks).toHaveLength(1)
    timers.shift()?.()
    expect(socks).toHaveLength(2)
    sock(socks, 1).open()
    expect(sock(socks, 1).frames).toEqual([{ op: 'subscribe' }])
    expect(backoffMs(0)).toBe(500)
    expect(backoffMs(20)).toBe(15_000)
  })

  it('signOut posts with the CSRF header', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ endSessionUrl: 'https://idp/out' }),
    })
    const { e } = env({ fetch: fetchMock as unknown as typeof fetch })
    const t = new HostedTransport(e, session)
    await expect(t.signOut()).resolves.toBe('https://idp/out')
    const init = fetchMock.mock.calls[0]?.[1] as { headers: Record<string, string> }
    expect(init.headers['X-CSRF-Token']).toBe('tok')
  })
})
