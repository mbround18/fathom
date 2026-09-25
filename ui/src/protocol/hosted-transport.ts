/**
 * Hosted-server transport (spec 031, T029; contracts/hosted-http-api.md).
 *
 * - Same-origin `ws(s)://<host>/ws`, authenticated by the HttpOnly session cookie: no token
 *   ever appears in a URL, script or storage.
 * - `GET /api/session` bootstraps the page (CSRF token for `POST /auth/logout`, expiry). A 401
 *   sends the browser to the sign-in URL preserving the return path.
 * - Close 4401 (session ended) redirects to sign-in; 4403 / 4429 / 1001-with-no-retry are
 *   terminal and reported through `onGone`; any other drop reconnects with capped
 *   exponential backoff and re-subscribes.
 * - Frames are the dev-bridge JSON frames; replies arrive in request order (a FIFO).
 */
import type { Transport } from './transport'
import type {
  BackendCmd,
  BridgeError,
  DevBridgeClientMsg,
  DevBridgeServerMsg,
  EventBatch,
  SubscriptionInfo,
} from './types'
import { BridgeFailure, type WebSocketFactory, type WebSocketLike } from './ws-transport'

export const CLOSE_SESSION_ENDED = 4401
export const CLOSE_ORIGIN = 4403
export const CLOSE_TAB_LIMIT = 4429

const BACKEND_GONE: BridgeError = { type: 'BackendGone' }
const OPEN = 1

export interface HostedSessionInfo {
  username: string
  groups: string[]
  csrf: string
  expiresAt: string
  idleTimeoutSec: number
  accessModel: string
  mode: 'hosted'
}

export interface HostedEnv {
  origin: string
  pathname: string
  search: string
  hash: string
  fetch: typeof fetch
  factory: WebSocketFactory
  redirect: (url: string) => void
  setTimer: (fn: () => void, ms: number) => unknown
  clearTimer: (h: unknown) => void
}

export function browserEnv(): HostedEnv {
  const loc = globalThis.location
  return {
    origin: loc.origin,
    pathname: loc.pathname,
    search: loc.search,
    hash: loc.hash,
    fetch: (...args) => globalThis.fetch(...args),
    factory: (u) => new WebSocket(u),
    redirect: (url) => {
      loc.assign(url)
    },
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (h) => {
      clearTimeout(h as ReturnType<typeof setTimeout>)
    },
  }
}

/** `wss://host/ws` for an https page, `ws://` for http (local dev). */
export function hostedWsUrl(origin: string): string {
  const url = new URL('/ws', origin)
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
  return url.toString()
}

/** Sign-in URL carrying a same-origin return path (never an absolute URL). */
export function signInUrl(loginUrl: string, pathname: string, search: string, hash = ''): string {
  const safeLogin =
    loginUrl.startsWith('/') && !loginUrl.startsWith('//') ? loginUrl : '/auth/login'
  const ret = `${pathname}${search}${hash}`
  return `${safeLogin}?return=${encodeURIComponent(ret.startsWith('/') ? ret : '/')}`
}

/** Backoff delay for the nth (0-based) reconnect attempt: 0.5s doubling, capped at 15s. */
export function backoffMs(attempt: number): number {
  return Math.min(500 * 2 ** attempt, 15_000)
}

type Pending =
  | { op: 'send'; resolve: (id: number) => void; reject: (err: BridgeFailure) => void }
  | {
      op: 'subscribe'
      resolve: (i: SubscriptionInfo) => void
      reject: (err: BridgeFailure) => void
    }

export class HostedTransport implements Transport {
  private socket: WebSocketLike | undefined
  private pending: Pending[] = []
  private outbox: string[] = []
  private onBatch: ((batch: EventBatch) => void) | undefined
  private subscribed = false
  private gone: BridgeError | undefined
  private readonly goneListeners = new Set<(err: BridgeError) => void>()
  private attempt = 0
  private timer: unknown
  private csrfToken: string

  private readonly env: HostedEnv
  readonly session: HostedSessionInfo

  constructor(env: HostedEnv, session: HostedSessionInfo) {
    this.env = env
    this.session = session
    this.csrfToken = session.csrf
    this.connect()
  }

  get csrf(): string {
    return this.csrfToken
  }

  send(cmd: BackendCmd): Promise<number> {
    return new Promise((resolve, reject) => {
      if (this.gone) {
        reject(new BridgeFailure(this.gone))
        return
      }
      this.pending.push({ op: 'send', resolve, reject })
      this.write({ op: 'send', cmd })
    })
  }

  subscribe(onBatch: (batch: EventBatch) => void): Promise<SubscriptionInfo> {
    return new Promise((resolve, reject) => {
      if (this.gone) {
        reject(new BridgeFailure(this.gone))
        return
      }
      this.onBatch = onBatch
      this.subscribed = true
      this.pending.push({ op: 'subscribe', resolve, reject })
      this.write({ op: 'subscribe' })
    })
  }

  onGone(listener: (err: BridgeError) => void): void {
    const { gone } = this
    if (gone) queueMicrotask(() => listener(gone))
    else this.goneListeners.add(listener)
  }

  /** `POST /auth/logout` with the CSRF token; resolves to the RP-initiated logout URL. */
  async signOut(): Promise<string | null> {
    const res = await this.env.fetch('/auth/logout', {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'X-CSRF-Token': this.csrfToken },
    })
    if (!res.ok) throw new Error(`sign-out failed (${res.status})`)
    const body = (await res.json()) as { endSessionUrl?: string | null }
    this.close()
    return body.endSessionUrl ?? null
  }

  close(): void {
    this.env.clearTimer(this.timer)
    this.socket?.close()
    this.terminate()
  }

  private connect(): void {
    const socket = this.env.factory(hostedWsUrl(this.env.origin))
    this.socket = socket
    socket.onopen = () => {
      this.attempt = 0
      const frames = this.outbox
      this.outbox = []
      for (const f of frames) socket.send(f)
    }
    socket.onmessage = (ev) => {
      this.receive(ev.data)
    }
    socket.onerror = () => undefined
    socket.onclose = (ev) => {
      if (socket !== this.socket) return
      this.onClosed(ev.code)
    }
  }

  private onClosed(code: number): void {
    if (this.gone) return
    if (code === CLOSE_SESSION_ENDED) {
      this.terminate()
      this.env.redirect(signInUrl('/auth/login', this.env.pathname, this.env.search, this.env.hash))
      return
    }
    if (code === CLOSE_ORIGIN || code === CLOSE_TAB_LIMIT) {
      this.terminate()
      return
    }
    // Transient drop: fail in-flight calls, back off, reconnect and re-subscribe.
    for (const w of this.pending.splice(0)) w.reject(new BridgeFailure(BACKEND_GONE))
    this.outbox = []
    const delay = backoffMs(this.attempt++)
    this.timer = this.env.setTimer(() => {
      if (this.gone) return
      this.connect()
      if (this.subscribed) {
        this.pending.push({ op: 'subscribe', resolve: () => undefined, reject: () => undefined })
        this.write({ op: 'subscribe' })
      }
    }, delay)
  }

  private write(msg: DevBridgeClientMsg): void {
    const frame = JSON.stringify(msg)
    if (this.socket?.readyState === OPEN) this.socket.send(frame)
    else this.outbox.push(frame)
  }

  private receive(data: unknown): void {
    let msg: DevBridgeServerMsg
    try {
      msg = JSON.parse(String(data)) as DevBridgeServerMsg
    } catch {
      console.error('hosted: ignoring a malformed frame')
      return
    }
    if (msg.op === 'batch') {
      this.onBatch?.(msg.batch)
      return
    }
    const waiter = this.pending.shift()
    if (!waiter) return
    if (msg.op === 'error') waiter.reject(new BridgeFailure(msg.error))
    else if (msg.op === 'sent' && waiter.op === 'send') waiter.resolve(msg.request_id)
    else if (msg.op === 'subscribed' && waiter.op === 'subscribe') waiter.resolve(msg.info)
    else {
      waiter.reject(new BridgeFailure(BACKEND_GONE))
      this.close()
    }
  }

  private terminate(): void {
    if (this.gone) return
    this.gone = BACKEND_GONE
    this.outbox = []
    for (const w of this.pending.splice(0)) w.reject(new BridgeFailure(BACKEND_GONE))
    for (const l of this.goneListeners) l(BACKEND_GONE)
    this.goneListeners.clear()
  }
}

/**
 * Bootstrap: `GET /api/session`. On 401 the browser is sent to sign-in and the returned promise
 * never settles (the page is navigating away).
 */
export async function createHostedTransport(env: HostedEnv = browserEnv()): Promise<Transport> {
  const res = await env.fetch('/api/session', {
    credentials: 'same-origin',
    headers: { Accept: 'application/json' },
  })
  if (res.status === 401) {
    let loginUrl = '/auth/login'
    try {
      const body = (await res.json()) as { loginUrl?: string }
      if (body.loginUrl) loginUrl = body.loginUrl
    } catch {
      /* default */
    }
    env.redirect(signInUrl(loginUrl, env.pathname, env.search, env.hash))
    return new Promise<Transport>(() => undefined)
  }
  if (!res.ok) throw new Error(`session bootstrap failed (${res.status})`)
  const session = (await res.json()) as HostedSessionInfo
  return new HostedTransport(env, session)
}
