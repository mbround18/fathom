/**
 * Dev WebSocket transport (`VITE_TRANSPORT=ws`, the live e2e suite): the same two calls as the
 * Tauri bridge, spoken to `tools/devbridge` as JSON frames (`DevBridgeClientMsg` /
 * `DevBridgeServerMsg`).
 *
 * - The bridge URL and token come from the page's query string (`?bridge=ws://…&token=…`) so
 *   the token never ends up in a bundle; `VITE_DEVBRIDGE_URL` is only a fallback for the URL.
 * - Replies to `send`/`subscribe` arrive in request order, so pending calls are a FIFO.
 * - A `Forbidden` reply (the bridge's write guard) rejects that one `send` with a
 *   `BridgeFailure` carrying `{ type: 'Forbidden', data: reason }`; the connection stays up.
 * - A closed socket is the Tauri "backend gone" path: every pending and later call rejects with
 *   a `BridgeFailure` of `{ type: 'BackendGone' }`, and `onGone` listeners fire once.
 *
 * Rejections are `Error`s that also carry the `BridgeError` fields (`type`, `data`), so
 * `describeBridgeError` reads them exactly like the plain objects Tauri's `invoke` rejects with.
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

const BACKEND_GONE: BridgeError = { type: 'BackendGone' }

/** A rejected bridge call: an `Error` with the `BridgeError`'s `type` (and `data`) on it. */
export class BridgeFailure extends Error {
  readonly type: BridgeError['type']
  readonly data: string | undefined

  constructor(error: BridgeError) {
    super(error.type === 'Forbidden' ? `Forbidden: ${error.data}` : error.type)
    this.name = 'BridgeFailure'
    this.type = error.type
    this.data = error.type === 'Forbidden' ? error.data : undefined
  }
}

/**
 * The bridge URL with its `token` query parameter. Query params (`bridge`, `token`) win over
 * `VITE_DEVBRIDGE_URL`; throws with a usable message when neither names a bridge.
 */
export function resolveBridgeUrl(
  search: string = globalThis.location.search,
  envUrl: string | undefined = import.meta.env.VITE_DEVBRIDGE_URL,
): string {
  const params = new URLSearchParams(search)
  const base = params.get('bridge') ?? envUrl
  if (!base) {
    throw new Error(
      'No dev bridge configured: open the app with ?bridge=ws://127.0.0.1:18290/&token=<token> ' +
        '(or set VITE_DEVBRIDGE_URL).',
    )
  }
  let url: URL
  try {
    url = new URL(base)
  } catch {
    throw new Error(`Invalid dev bridge URL: ${base}`)
  }
  if (url.protocol !== 'ws:' && url.protocol !== 'wss:') {
    throw new Error(`The dev bridge URL must be ws:// or wss://, got ${base}`)
  }
  const token = params.get('token')
  if (token !== null) url.searchParams.set('token', token)
  return url.toString()
}

type Pending =
  | { op: 'send'; resolve: (id: number) => void; reject: (err: BridgeFailure) => void }
  | {
      op: 'subscribe'
      resolve: (info: SubscriptionInfo) => void
      reject: (err: BridgeFailure) => void
    }

/** The subset of the DOM `WebSocket` the transport uses (a mock in unit tests). */
export interface WebSocketLike {
  readonly readyState: number
  onopen: ((ev: Event) => void) | null
  onmessage: ((ev: MessageEvent) => void) | null
  onclose: ((ev: CloseEvent) => void) | null
  onerror: ((ev: Event) => void) | null
  send(data: string): void
  close(code?: number, reason?: string): void
}

export type WebSocketFactory = (url: string) => WebSocketLike

const OPEN = 1

export class WsTransport implements Transport {
  private readonly socket: WebSocketLike
  private readonly pending: Pending[] = []
  /** Frames written before the socket opened. */
  private outbox: string[] = []
  private onBatch: ((batch: EventBatch) => void) | undefined
  private gone: BridgeError | undefined
  private readonly goneListeners = new Set<(err: BridgeError) => void>()

  constructor(url: string, factory: WebSocketFactory = (u) => new WebSocket(u)) {
    this.socket = factory(url)
    this.socket.onopen = () => {
      const frames = this.outbox
      this.outbox = []
      for (const frame of frames) this.socket.send(frame)
    }
    this.socket.onmessage = (ev) => {
      this.receive(ev.data)
    }
    // `error` is always followed by `close`; handle the end of the connection there.
    this.socket.onerror = () => undefined
    this.socket.onclose = () => {
      this.shutdown()
    }
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
      this.pending.push({ op: 'subscribe', resolve, reject })
      this.write({ op: 'subscribe' })
    })
  }

  /** Called once when the connection ends (immediately if it already has). */
  onGone(listener: (err: BridgeError) => void): void {
    const { gone } = this
    if (gone) queueMicrotask(() => listener(gone))
    else this.goneListeners.add(listener)
  }

  /** Close the connection (tests, page teardown). */
  close(): void {
    this.socket.close()
    this.shutdown()
  }

  private write(msg: DevBridgeClientMsg): void {
    const frame = JSON.stringify(msg)
    if (this.socket.readyState === OPEN) this.socket.send(frame)
    else this.outbox.push(frame)
  }

  private receive(data: unknown): void {
    let msg: DevBridgeServerMsg
    try {
      msg = JSON.parse(String(data)) as DevBridgeServerMsg
    } catch {
      console.error('devbridge: ignoring a malformed frame', data)
      return
    }
    if (msg.op === 'batch') {
      this.onBatch?.(msg.batch)
      return
    }
    const waiter = this.pending.shift()
    if (!waiter) {
      console.error('devbridge: reply with no pending request', msg)
      return
    }
    if (msg.op === 'error') {
      waiter.reject(new BridgeFailure(msg.error))
    } else if (msg.op === 'sent' && waiter.op === 'send') {
      waiter.resolve(msg.request_id)
    } else if (msg.op === 'subscribed' && waiter.op === 'subscribe') {
      waiter.resolve(msg.info)
    } else {
      // Out of step with the bridge: nothing after this can be trusted.
      console.error(`devbridge: got '${msg.op}' while awaiting '${waiter.op}'`)
      waiter.reject(new BridgeFailure(BACKEND_GONE))
      this.close()
    }
  }

  private shutdown(): void {
    if (this.gone) return
    this.gone = BACKEND_GONE
    this.outbox = []
    for (const waiter of this.pending.splice(0)) waiter.reject(new BridgeFailure(BACKEND_GONE))
    for (const listener of this.goneListeners) listener(BACKEND_GONE)
    this.goneListeners.clear()
  }
}

declare global {
  interface Window {
    /**
     * `ws` mode only: the live transport, so live e2e specs can issue a raw `send` (e.g. to
     * prove the bridge's write guard) with `page.evaluate`. The bridge still guards it.
     */
    __fathomDevBridge?: Pick<WsTransport, 'send'>
  }
}

/** The transport for this page, from `?bridge=&token=` or `VITE_DEVBRIDGE_URL`. */
export function createWsTransport(): WsTransport {
  const transport = new WsTransport(resolveBridgeUrl())
  window.__fathomDevBridge = transport
  return transport
}
