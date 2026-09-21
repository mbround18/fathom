/** WsTransport over a mock WebSocket: framing, FIFO replies, Forbidden, and backend-gone. */
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { BackendCmd, DevBridgeServerMsg, EventBatch } from '@/protocol/types'
import { connectBackend } from '@/protocol/use-backend'
import {
  BridgeFailure,
  resolveBridgeUrl,
  WsTransport,
  type WebSocketLike,
} from '@/protocol/ws-transport'
import { initialData, useStore } from '@/store'

class MockSocket implements WebSocketLike {
  readyState = 0
  onopen: ((ev: Event) => void) | null = null
  onmessage: ((ev: MessageEvent) => void) | null = null
  onclose: ((ev: CloseEvent) => void) | null = null
  onerror: ((ev: Event) => void) | null = null
  readonly frames: unknown[] = []
  readonly url: string

  constructor(url: string) {
    this.url = url
  }

  send(data: string): void {
    if (this.readyState !== 1) throw new Error('send before open')
    this.frames.push(JSON.parse(data))
  }
  close(): void {
    this.drop()
  }

  // ---- test controls
  open(): void {
    this.readyState = 1
    this.onopen?.(new Event('open'))
  }
  reply(msg: DevBridgeServerMsg): void {
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(msg) }))
  }
  drop(): void {
    if (this.readyState === 3) return
    this.readyState = 3
    this.onerror?.(new Event('error'))
    this.onclose?.(new CloseEvent('close'))
  }
}

function setup(): { transport: WsTransport; socket: MockSocket } {
  let socket: MockSocket | undefined
  const transport = new WsTransport('ws://127.0.0.1:18290/?token=t', (url) => {
    socket = new MockSocket(url)
    return socket
  })
  if (!socket) throw new Error('factory not called')
  return { transport, socket }
}

const INFO = { protocolVersion: 1, backendStarted: true } as const
const REFRESH: BackendCmd = { type: 'RefreshPods' }
const EMPTY_BATCH: EventBatch = { envelopes: [], dropped: 0 }

describe('resolveBridgeUrl', () => {
  it('builds the URL from ?bridge and ?token, encoding the token', () => {
    expect(resolveBridgeUrl('?bridge=ws://127.0.0.1:18290/&token=a%2Bb%3D', undefined)).toBe(
      'ws://127.0.0.1:18290/?token=a%2Bb%3D',
    )
  })

  it('prefers the query over VITE_DEVBRIDGE_URL, which is only a fallback', () => {
    expect(resolveBridgeUrl('?bridge=ws://127.0.0.1:1/&token=q', 'ws://127.0.0.1:2/')).toBe(
      'ws://127.0.0.1:1/?token=q',
    )
    expect(resolveBridgeUrl('?token=q', 'ws://127.0.0.1:2/?token=env')).toBe(
      'ws://127.0.0.1:2/?token=q',
    )
    expect(resolveBridgeUrl('', 'ws://127.0.0.1:2/?token=env')).toBe('ws://127.0.0.1:2/?token=env')
  })

  it('throws a usable message without a bridge, or with a non-ws URL', () => {
    expect(() => resolveBridgeUrl('', undefined)).toThrow(/\?bridge=ws:/)
    expect(() => resolveBridgeUrl('?bridge=http://localhost/', undefined)).toThrow(/ws:\/\//)
    expect(() => resolveBridgeUrl('?bridge=nope', undefined)).toThrow(/Invalid/)
  })
})

describe('WsTransport', () => {
  it('queues frames until the socket opens, then resolves replies in request order', async () => {
    const { transport, socket } = setup()
    const batches: EventBatch[] = []
    const sub = transport.subscribe((b) => batches.push(b))
    const first = transport.send(REFRESH)
    const second = transport.send({ type: 'RefreshDeployments' })
    expect(socket.frames).toEqual([])

    socket.open()
    expect(socket.frames).toEqual([
      { op: 'subscribe' },
      { op: 'send', cmd: REFRESH },
      { op: 'send', cmd: { type: 'RefreshDeployments' } },
    ])

    socket.reply({ op: 'subscribed', info: INFO })
    socket.reply({ op: 'batch', batch: EMPTY_BATCH })
    socket.reply({ op: 'sent', request_id: 7 })
    socket.reply({ op: 'sent', request_id: 8 })
    await expect(sub).resolves.toEqual(INFO)
    await expect(first).resolves.toBe(7)
    await expect(second).resolves.toBe(8)
    expect(batches).toEqual([EMPTY_BATCH])
  })

  it('rejects only the refused send with the Forbidden reason; later sends still work', async () => {
    const { transport, socket } = setup()
    socket.open()
    const refused = transport.send({ type: 'DeletePod', data: { namespace: 'default', name: 'x' } })
    const ok = transport.send(REFRESH)
    socket.reply({ op: 'error', error: { type: 'Forbidden', data: 'DeletePod writes outside' } })
    socket.reply({ op: 'sent', request_id: 2 })
    await expect(refused).rejects.toBeInstanceOf(BridgeFailure)
    await expect(refused).rejects.toMatchObject({
      type: 'Forbidden',
      data: 'DeletePod writes outside',
    })
    await expect(ok).resolves.toBe(2)
  })

  it('a closed socket rejects pending and later calls with BackendGone and fires onGone once', async () => {
    const { transport, socket } = setup()
    socket.open()
    const gone = vi.fn()
    transport.onGone(gone)
    const pending = transport.send(REFRESH)
    socket.drop()
    await expect(pending).rejects.toMatchObject({ type: 'BackendGone' })
    await expect(transport.send(REFRESH)).rejects.toMatchObject({ type: 'BackendGone' })
    await expect(transport.subscribe(() => undefined)).rejects.toMatchObject({
      type: 'BackendGone',
    })
    expect(gone).toHaveBeenCalledExactlyOnceWith({ type: 'BackendGone' })

    const late = vi.fn()
    transport.onGone(late)
    await Promise.resolve()
    expect(late).toHaveBeenCalledOnce()
  })

  it('a socket that never opens (bad token / origin) fails subscribe as BackendGone', async () => {
    const { transport, socket } = setup()
    const sub = transport.subscribe(() => undefined)
    socket.drop()
    await expect(sub).rejects.toMatchObject({ type: 'BackendGone' })
  })

  it('ignores malformed frames and treats an out-of-step reply as fatal', async () => {
    const { transport, socket } = setup()
    socket.open()
    const errors = vi.spyOn(console, 'error').mockImplementation(() => undefined)
    socket.onmessage?.(new MessageEvent('message', { data: 'not json' }))
    const sent = transport.send(REFRESH)
    socket.reply({ op: 'subscribed', info: INFO })
    await expect(sent).rejects.toMatchObject({ type: 'BackendGone' })
    expect(errors).toHaveBeenCalledTimes(2)
    await expect(transport.send(REFRESH)).rejects.toMatchObject({ type: 'BackendGone' })
  })
})

describe('connectBackend over WsTransport', () => {
  beforeEach(() => {
    useStore.setState(initialData())
  })

  async function connected(): Promise<{ transport: WsTransport; socket: MockSocket }> {
    const ctx = setup()
    ctx.socket.open()
    const done = connectBackend(ctx.transport)
    ctx.socket.reply({ op: 'subscribed', info: INFO })
    await done
    // Acknowledge the bootstrap sends.
    for (let id = 1; id <= 3; id++) ctx.socket.reply({ op: 'sent', request_id: id })
    await Promise.resolve()
    return ctx
  }

  it('a Forbidden request rejects with the reason but leaves the connection ready', async () => {
    const { socket } = await connected()
    expect(useStore.getState().conn.status).toBe('ready')

    const req = useStore
      .getState()
      .request({ type: 'DeletePod', data: { namespace: 'kube-system', name: 'x' } })
    socket.reply({
      op: 'error',
      error: {
        type: 'Forbidden',
        data: "DeletePod writes outside the sandbox namespace 'fathom-dev'",
      },
    })
    await expect(req).rejects.toThrow(/Forbidden.*outside the sandbox namespace 'fathom-dev'/)
    expect(useStore.getState().conn.status).toBe('ready')
  })

  it('the socket closing moves the connection to backend-gone without a send', async () => {
    const { socket } = await connected()
    socket.drop()
    const { conn } = useStore.getState()
    expect(conn.status).toBe('backend-gone')
    expect(conn.fatal).toMatch(/BackendGone/)
  })
})
