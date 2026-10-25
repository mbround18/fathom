/**
 * The Depths WebSocket (contracts/protocol.md, "Transport").
 *
 * - Same-origin `ws(s)://<host>/api/v1/ws`, derived from `location`: `wss:` on an https page.
 *   The session is the HttpOnly cookie; no token ever goes in the URL.
 * - The first frame on every socket, including each reconnect, is `Hello{protocol_version,
 *   csrf_token}`. Commands sent before that are queued and flushed right after it.
 * - Close 4401 (no session) and 4403 (forbidden: wrong CSRF token, or access removed) and 4429
 *   (too many sockets) are terminal and reported through `onTerminal`. Any other drop reconnects
 *   with capped exponential backoff, and `onStatus('open', { reconnected: true })` tells feature
 *   stores to re-issue what they were watching.
 * - Server frames are `EventEnvelope`s, handed to `onEvent` unchanged. Every string in them is
 *   untrusted: the UI renders them as text, never as markup.
 */
import {
  CloseCode,
  PROTOCOL_VERSION,
  type Cmd,
  type CmdEnvelope,
  type EventEnvelope,
  type Hello,
  type RequestId,
} from './generated'

/** The subset of `WebSocket` the transport uses, so tests can hand it a fake. */
export interface WebSocketLike {
  readonly readyState: number
  onopen: ((ev: unknown) => void) | null
  onmessage: ((ev: { data: unknown }) => void) | null
  onerror: ((ev: unknown) => void) | null
  onclose: ((ev: { code: number }) => void) | null
  send(data: string): void
  close(code?: number): void
}

export type WebSocketFactory = (url: string) => WebSocketLike

export interface TransportEnv {
  /** The page origin, e.g. `https://depths.example.com`. */
  origin: string
  factory: WebSocketFactory
  setTimer: (fn: () => void, ms: number) => unknown
  clearTimer: (handle: unknown) => void
}

export type ConnectionStatus = 'connecting' | 'open' | 'reconnecting' | 'closed'

/** Why the socket closed for good. */
export type TerminalReason = 'unauthenticated' | 'forbidden' | 'too_many'

export interface TransportHandlers {
  onEvent: (envelope: EventEnvelope) => void
  onStatus?: (status: ConnectionStatus, info: { reconnected: boolean; attempt: number }) => void
  onTerminal?: (reason: TerminalReason) => void
}

const OPEN = 1

export function browserTransportEnv(): TransportEnv {
  return {
    origin: globalThis.location.origin,
    factory: (u) => new WebSocket(u) as unknown as WebSocketLike,
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (h) => {
      clearTimeout(h as ReturnType<typeof setTimeout>)
    },
  }
}

/** `wss://host/api/v1/ws` for an https page, `ws://` for http (local dev). */
export function wsUrl(origin: string): string {
  const url = new URL('/api/v1/ws', origin)
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
  return url.toString()
}

/** Delay before the nth (0-based) reconnect: 0.5 s doubling, capped at 15 s. */
export function backoffMs(attempt: number): number {
  return Math.min(500 * 2 ** attempt, 15_000)
}

export function terminalReason(code: number): TerminalReason | null {
  switch (code) {
    case CloseCode.Unauthenticated:
      return 'unauthenticated'
    case CloseCode.Forbidden:
      return 'forbidden'
    case CloseCode.TooMany:
      return 'too_many'
    default:
      return null
  }
}

export class Transport {
  private socket: WebSocketLike | undefined
  private outbox: string[] = []
  private nextId: RequestId = 1
  private attempt = 0
  private everOpened = false
  private timer: unknown
  private closed = false
  private helloSent = false

  private readonly env: TransportEnv
  private readonly csrfToken: string
  private readonly handlers: TransportHandlers

  constructor(env: TransportEnv, csrfToken: string, handlers: TransportHandlers) {
    this.env = env
    this.csrfToken = csrfToken
    this.handlers = handlers
    this.connect()
  }

  /** Send a command; its events come back with `reply_to` equal to the returned id. */
  send(cmd: Cmd): RequestId {
    const id = this.nextId++
    if (this.closed) return id
    const envelope: CmdEnvelope = { id, cmd }
    const frame = JSON.stringify(envelope)
    if (this.socket?.readyState === OPEN && this.helloSent) this.socket.send(frame)
    else this.outbox.push(frame)
    return id
  }

  /** Close for good (sign-out, page teardown). No reconnect follows. */
  close(): void {
    if (this.closed) return
    this.closed = true
    this.env.clearTimer(this.timer)
    this.outbox = []
    const s = this.socket
    this.socket = undefined
    s?.close(1000)
    this.handlers.onStatus?.('closed', { reconnected: false, attempt: this.attempt })
  }

  private connect(): void {
    this.helloSent = false
    if (!this.everOpened) this.handlers.onStatus?.('connecting', { reconnected: false, attempt: 0 })
    const socket = this.env.factory(wsUrl(this.env.origin))
    this.socket = socket
    socket.onopen = () => {
      if (socket !== this.socket) return
      const hello: Hello = { protocol_version: PROTOCOL_VERSION, csrf_token: this.csrfToken }
      socket.send(JSON.stringify(hello))
      this.helloSent = true
      const frames = this.outbox
      this.outbox = []
      for (const f of frames) socket.send(f)
      const reconnected = this.everOpened
      this.everOpened = true
      this.attempt = 0
      this.handlers.onStatus?.('open', { reconnected, attempt: 0 })
    }
    socket.onmessage = (ev) => {
      if (socket !== this.socket) return
      this.receive(ev.data)
    }
    socket.onerror = () => undefined
    socket.onclose = (ev) => {
      if (socket !== this.socket) return
      this.onClosed(ev.code)
    }
  }

  private onClosed(code: number): void {
    if (this.closed) return
    const reason = terminalReason(code)
    if (reason) {
      this.closed = true
      this.socket = undefined
      this.outbox = []
      this.handlers.onStatus?.('closed', { reconnected: false, attempt: this.attempt })
      this.handlers.onTerminal?.(reason)
      return
    }
    // A transient drop: whatever was in flight is lost. Commands sent while down are queued and
    // go out after the next Hello; stores re-issue their watches on `reconnected`.
    this.socket = undefined
    this.helloSent = false
    const delay = backoffMs(this.attempt++)
    this.handlers.onStatus?.('reconnecting', { reconnected: false, attempt: this.attempt })
    this.timer = this.env.setTimer(() => {
      if (!this.closed) this.connect()
    }, delay)
  }

  private receive(data: unknown): void {
    if (typeof data !== 'string') return
    let parsed: unknown
    try {
      parsed = JSON.parse(data)
    } catch {
      console.error('depths: ignoring a malformed frame')
      return
    }
    if (!isEnvelope(parsed)) return
    this.handlers.onEvent(parsed)
  }
}

function isEnvelope(v: unknown): v is EventEnvelope {
  if (typeof v !== 'object' || v === null || !('event' in v)) return false
  const ev: unknown = v.event
  return typeof ev === 'object' && ev !== null && 'type' in ev && typeof ev.type === 'string'
}
