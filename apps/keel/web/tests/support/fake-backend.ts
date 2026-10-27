/**
 * The fake Keel backend every Playwright story runs against (spec 055 T031), after the Depths one.
 *
 * It is installed on a page with Playwright's network interception, so the app under test is the
 * real production bundle and holds no fake mode of its own:
 *
 * - The session is the readable CSRF cookie (`keel_csrf`, as under `--insecure-dev`) plus `me`.
 *   The `backend` fixture sets the cookie; `signedOut()` clears both.
 * - `GET /auth/login` signs `loginAs` in: sets the cookie and redirects to `/`, as the provider's
 *   callback does. `POST /auth/logout` checks `X-CSRF-Token`, clears the cookie and answers
 *   `{ endSessionUrl: null }`. Both are recorded in `http`.
 * - Any other `/api/**` is a 404, so nothing reaches a real server.
 * - `/api/v1/ws` speaks the protocol: it checks `Hello` and closes with apps/keel/server/src/ws.rs's
 *   codes (4401 no session or a bad Hello, 4403 CSRF mismatch, 4400 another protocol version), then
 *   answers each `CmdEnvelope` with `EventEnvelope`s. `GetMe` and the passkey commands have default
 *   answers; `on(type, handler)` replaces one, and `replay(envelopes)` sends scripted envelopes as is.
 *
 * Usage, in a spec:
 *
 * ```ts
 * import { expect, test } from '../../support/fake-backend'
 *
 * test('signs in', async ({ page, backend }) => {
 *   await page.goto('/runs')
 *   await backend.waitForCmd('GetMe')
 *   backend.replay([{ event: { type: 'Revoked' } }])
 * })
 * ```
 *
 * Nothing here sleeps or reads the wall clock for behaviour: events arrive when a command does or
 * when the test sends them, so stories stay deterministic.
 */
import {
  test as base,
  expect,
  type BrowserContext,
  type Page,
  type Request,
  type WebSocketRoute,
} from '@playwright/test'

import type {
  Cmd,
  CmdEnvelope,
  Event,
  EventEnvelope,
  Hello,
  Me,
  PasskeyInfo,
  Verification,
} from '../../src/protocol/wire'
import { PROTOCOL_VERSION } from '../../src/protocol/generated/constants'

export { expect }

/** The server's close codes (apps/keel/server/src/ws.rs). */
export const Close = {
  Outdated: 4400,
  Unauthenticated: 4401,
  Csrf: 4403,
  GoingAway: 1001,
} as const

/** The CSRF cookie the server sets under `--insecure-dev` (plain HTTP, as the preview is). */
export const CSRF_COOKIE = 'keel_csrf'

export type CmdType = Cmd['type']
export type CmdOf<T extends CmdType> = Extract<Cmd, { type: T }>

/** What a scripted handler returns: events to send in reply, or `undefined` to fall through. */
export type Reply = Event[] | undefined

export interface HandlerContext {
  backend: FakeBackend
  socket: FakeSocket
  id: number
}

export type Handler<T extends CmdType> = (cmd: CmdOf<T>, ctx: HandlerContext) => Reply

/** One HTTP request the app made to the fake, as recorded. */
export interface HttpCall {
  method: string
  path: string
  csrf: string | null
}

/** One open mocked socket. */
export class FakeSocket {
  readonly hellos: unknown[] = []
  readonly received: CmdEnvelope[] = []
  ready = false
  closed: number | null = null

  private readonly route: WebSocketRoute

  constructor(route: WebSocketRoute) {
    this.route = route
  }

  sendEnvelope(env: EventEnvelope): void {
    if (this.closed !== null) return
    this.route.send(JSON.stringify(env))
  }

  send(event: Event, replyTo?: number): void {
    this.sendEnvelope(replyTo === undefined ? { event } : { reply_to: replyTo, event })
  }

  close(code: number, reason = ''): void {
    if (this.closed !== null) return
    this.closed = code
    void this.route.close({ code, reason })
  }
}

/** base64url of a UTF-8 string, for the canned WebAuthn options. */
function b64url(s: string): string {
  return Buffer.from(s, 'utf8').toString('base64url')
}

export const fixtures = {
  verification: (over: Partial<Verification> = {}): Verification => ({
    verified: false,
    at: null,
    window_secs: 900,
    enrolled: false,
    required: false,
    recovering: false,
    ...over,
  }),

  me: (over: Partial<Me> = {}): Me => ({
    id: '01J0000000000000000000ADA',
    display_name: 'Ada Lovelace',
    email: 'ada@example.com',
    instance_role: 'admin',
    grants: [],
    verification: fixtures.verification(),
    ...over,
  }),
}

/** The relying party the canned options name; matches the preview's origin. */
const RP_ID = 'localhost'

export class FakeBackend {
  /** `null` = no session: the socket closes 4401 on `Hello`. */
  me: Me | null = fixtures.me()
  /** Who `/auth/login` signs in. */
  loginAs: Me = fixtures.me()
  /** The session's CSRF token, as the cookie carries it. */
  csrf = 'fake-csrf-token_0123456789'
  /** Passkeys registered through the fake, oldest first. */
  passkeys: PasskeyInfo[] = []
  /** Close code for every new socket's `Hello`, e.g. `Close.Outdated`. `null` = accept. */
  helloClose: number | null = null

  readonly sockets: FakeSocket[] = []
  readonly http: HttpCall[] = []

  private page: Page | null = null
  private baseURL = ''
  private readonly handlers = new Map<CmdType, Handler<CmdType>>()
  private readonly waiters: { type: CmdType; resolve: (c: Cmd) => void }[] = []
  private readonly awaited = new WeakSet<Cmd>()
  /** Open ceremonies by id, with what they were for. */
  private readonly ceremonies = new Map<string, { kind: 'register' | 'verify'; name: string }>()
  private seq = 0

  /** Installs the fake on `page`. Call before `page.goto`. */
  static async install(page: Page, baseURL: string, setup?: (b: FakeBackend) => void): Promise<FakeBackend> {
    const b = new FakeBackend()
    setup?.(b)
    await b.attach(page, baseURL)
    return b
  }

  async attach(page: Page, baseURL: string): Promise<void> {
    this.page = page
    this.baseURL = baseURL
    // Later routes take precedence: the catch-all first, so nothing reaches a real server.
    await page.route('**/api/**', (r) => r.fulfill({ status: 404, json: { error: 'not_found' } }))
    await page.route('**/auth/**', async (r) => r.fulfill(await this.auth(r.request())))
    await page.routeWebSocket(/\/api\/v1\/ws$/, (ws) => {
      this.accept(ws)
    })
    if (this.me !== null) await this.setCookie(page.context())
  }

  // --- Scripting ----------------------------------------------------------------------------

  /** No session: clears `me` and the cookie. Call before `page.goto`. */
  async signedOut(): Promise<void> {
    this.me = null
    await this.page?.context().clearCookies()
  }

  /** Replaces the default handling of one command. Return `undefined` to fall through to it. */
  on<T extends CmdType>(type: T, handler: Handler<T>): this {
    this.handlers.set(type, handler as unknown as Handler<CmdType>)
    return this
  }

  /** Sends a live event (no `reply_to`) on every open socket. */
  push(event: Event): void {
    for (const s of this.open()) s.send(event)
  }

  /** Sends scripted envelopes, exactly as given, on every open socket. */
  replay(envelopes: readonly EventEnvelope[]): void {
    for (const s of this.open()) for (const e of envelopes) s.sendEnvelope(e)
  }

  /** What the server does when the session's access changes (R11): `Revoked`, then 4401. */
  revoke(): void {
    this.me = null
    for (const s of this.open()) {
      s.send({ type: 'Revoked' })
      s.close(Close.Unauthenticated, 'revoked')
    }
  }

  /** Closes every open socket with `code`. */
  closeSockets(code: number = Close.Unauthenticated): void {
    for (const s of this.open()) s.close(code)
  }

  /** Every command received, across sockets, in order. */
  get received(): Cmd[] {
    return this.sockets.flatMap((s) => s.received.map((e) => e.cmd))
  }

  /** Every `Hello` frame received, across sockets. */
  get hellos(): unknown[] {
    return this.sockets.flatMap((s) => s.hellos)
  }

  /** Resolves with the next `type` command (or one already received and not yet awaited). */
  waitForCmd<T extends CmdType>(type: T): Promise<CmdOf<T>> {
    const seen = this.received.find((c): c is CmdOf<T> => c.type === type && !this.awaited.has(c))
    if (seen) {
      this.awaited.add(seen)
      return Promise.resolve(seen)
    }
    return new Promise((resolve) => {
      this.waiters.push({ type, resolve: resolve as (c: Cmd) => void })
    })
  }

  // --- HTTP ---------------------------------------------------------------------------------

  private async setCookie(context: BrowserContext): Promise<void> {
    await context.addCookies([{ name: CSRF_COOKIE, value: this.csrf, url: this.baseURL, sameSite: 'Strict' }])
  }

  private async auth(req: Request) {
    const url = new URL(req.url())
    this.http.push({ method: req.method(), path: url.pathname, csrf: req.headers()['x-csrf-token'] ?? null })
    if (url.pathname === '/auth/logout' && req.method() === 'POST') {
      if (this.me === null || req.headers()['x-csrf-token'] !== this.csrf) {
        return { status: 403, json: { error: 'csrf' } }
      }
      this.me = null
      await this.page?.context().clearCookies()
      return { status: 200, json: { endSessionUrl: null } }
    }
    if (url.pathname === '/auth/login' && req.method() === 'GET') {
      // The provider round trip, collapsed: the callback sets the session and lands on `/`.
      this.me = this.loginAs
      if (this.page) await this.setCookie(this.page.context())
      return { status: 302, headers: { location: '/' } }
    }
    return { status: 404, json: { error: 'not_found' } }
  }

  // --- WebSocket ----------------------------------------------------------------------------

  private open(): FakeSocket[] {
    return this.sockets.filter((s) => s.closed === null)
  }

  private accept(route: WebSocketRoute): void {
    const socket = new FakeSocket(route)
    this.sockets.push(socket)
    route.onClose(() => {
      socket.closed ??= 1000
    })
    route.onMessage((raw) => {
      let frame: unknown
      try {
        frame = JSON.parse(typeof raw === 'string' ? raw : raw.toString('utf8'))
      } catch {
        socket.close(Close.Unauthenticated, 'malformed frame')
        return
      }
      if (!socket.ready) this.hello(socket, frame)
      else this.command(socket, frame)
    })
  }

  private hello(socket: FakeSocket, frame: unknown): void {
    socket.hellos.push(frame)
    const h = frame as Partial<Hello> | null
    if (this.me === null) {
      socket.close(Close.Unauthenticated, 'unauthenticated')
    } else if (h?.protocol_version !== PROTOCOL_VERSION) {
      socket.close(Close.Outdated, 'protocol version')
    } else if (h.csrf_token !== this.csrf) {
      socket.close(Close.Csrf, 'csrf')
    } else if (this.helloClose !== null) {
      socket.close(this.helloClose)
    } else {
      socket.ready = true
    }
  }

  private command(socket: FakeSocket, frame: unknown): void {
    const env = frame as Partial<CmdEnvelope> | null
    if (typeof env?.id !== 'number' || typeof env.cmd?.type !== 'string') {
      socket.close(Close.Unauthenticated, 'expected a CmdEnvelope')
      return
    }
    const { id, cmd } = env as CmdEnvelope
    socket.received.push(env as CmdEnvelope)
    const w = this.waiters.findIndex((x) => x.type === cmd.type)
    if (w >= 0) {
      const [waiter] = this.waiters.splice(w, 1)
      this.awaited.add(cmd)
      waiter?.resolve(cmd)
    }
    if (this.me === null) {
      socket.close(Close.Unauthenticated)
      return
    }
    const scripted = this.handlers.get(cmd.type)?.(cmd, { backend: this, socket, id })
    for (const e of scripted ?? this.respond(this.me, cmd)) socket.send(e, id)
  }

  private respond(me: Me, cmd: Cmd): Event[] {
    switch (cmd.type) {
      case 'GetMe':
        return [{ type: 'Me', data: me }]
      case 'PasskeyBegin': {
        const ceremony = `cer-${String(++this.seq)}`
        const { purpose } = cmd.data
        this.ceremonies.set(ceremony, purpose.kind === 'register' ? { kind: 'register', name: purpose.name } : { kind: 'verify', name: '' })
        const challenge = b64url(`challenge-${ceremony}`)
        const options =
          purpose.kind === 'register'
            ? {
                publicKey: {
                  rp: { id: RP_ID, name: 'Keel' },
                  user: { id: b64url(me.id), name: me.email ?? me.id, displayName: me.display_name },
                  challenge,
                  pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
                  authenticatorSelection: { residentKey: 'required', userVerification: 'required' },
                  excludeCredentials: this.passkeys.map((p) => ({ type: 'public-key', id: p.id })),
                  timeout: 60_000,
                },
              }
            : {
                publicKey: {
                  challenge,
                  rpId: RP_ID,
                  userVerification: 'required',
                  allowCredentials: this.passkeys.map((p) => ({ type: 'public-key', id: p.id })),
                  timeout: 60_000,
                },
              }
        return [{ type: 'PasskeyChallenge', data: { ceremony, options } }]
      }
      case 'PasskeyFinish': {
        const c = this.ceremonies.get(cmd.data.ceremony)
        if (!c) return [{ type: 'PasskeyRefused', data: { reason: 'expired' } }]
        this.ceremonies.delete(cmd.data.ceremony)
        const verification = { ...me.verification, verified: true, at: '2026-10-27T12:00:00Z', recovering: false }
        if (c.kind === 'register') {
          const info: PasskeyInfo = { id: cmd.data.credential.id, name: c.name, created: '2026-10-27T12:00:00Z', last_used: null }
          this.passkeys.push(info)
          this.me = { ...me, verification: { ...verification, enrolled: true } }
          return [
            { type: 'PasskeyRegistered', data: info },
            { type: 'Verification', data: this.me.verification },
          ]
        }
        if (!this.passkeys.some((p) => p.id === cmd.data.credential.id)) {
          return [{ type: 'PasskeyRefused', data: { reason: 'unknown_credential' } }]
        }
        this.me = { ...me, verification }
        return [{ type: 'Verification', data: verification }]
      }
      case 'ListPasskeys':
        return [{ type: 'Passkeys', data: { items: this.passkeys } }]
      default:
        return [{ type: 'OpError', data: { code: 'unsupported', message: `the fake has no answer for ${cmd.type}` } }]
    }
  }
}

/**
 * `test` with a `backend` fixture always installed on `page`: signed in as an admin with no
 * passkey. Script it (or call `signedOut()`) before `page.goto`.
 */
export const test = base.extend<{ backend: FakeBackend }>({
  // `provide` rather than Playwright's usual `use`, which the React hooks lint rule reads as a hook.
  // `auto`, so a spec that never names `backend` still runs against it rather than nothing.
  backend: [
    async ({ page, baseURL }, provide) => {
      await provide(await FakeBackend.install(page, baseURL ?? 'http://localhost'))
    },
    { auto: true },
  ],
})
