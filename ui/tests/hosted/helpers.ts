/**
 * Helpers for the live hosted suite (spec 031, T038). See playwright.hosted.config.ts.
 *
 * - `signIn` drives the provider's real login page (auth-code + PKCE through fathom-server).
 * - `apiSession` reads `GET /api/session` from inside the page (browser DNS mapping + cookie).
 * - `RawWs` opens a second, same-origin `/ws` connection in the signed-in page (cookie auth,
 *   exact Origin) and speaks the dev-bridge frames directly, so a spec can force commands the
 *   UI would hide (e.g. alice's scale) and read the verbatim `CommandFinished` refusal.
 * - `kubectl` is pinned to the kind context of the hosted cluster and refuses anything else.
 */
import { execFileSync } from 'node:child_process'
import { expect, type Browser, type BrowserContext, type Page } from '@playwright/test'

import { PASSWORD, PROVIDER } from './provider'

export { PROVIDER }
export const CLUSTER = process.env.CLUSTER ?? `fathom-oidc-${PROVIDER}`
export const KUBE_CONTEXT = `kind-${CLUSTER}`
export const APP = 'https://fathom.localtest.me'
/** The chart's `claims.usernamePrefix`/`groupsPrefix` (mirrors the API server's OIDC flags). */
export const PREFIX = 'oidc:'
/** A user as fathom and the API server name them (claim value plus prefix). */
export const kubeUser = (u: string): string => `${PREFIX}${u}`

export function kubectl(...args: string[]): string {
  if (!KUBE_CONTEXT.startsWith('kind-'))
    throw new Error(`refusing non-kind context ${KUBE_CONTEXT}`)
  return execFileSync('kubectl', ['--context', KUBE_CONTEXT, ...args], {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  })
}

/** A fresh browser context (no provider SSO cookie) with the suite's `use` options. */
export async function newUserContext(browser: Browser): Promise<BrowserContext> {
  return browser.newContext({
    baseURL: APP,
    ignoreHTTPSErrors: true,
    viewport: { width: 1600, height: 900 },
  })
}

/** Fill the provider's login form (the page is already on it). */
export async function providerLogin(page: Page, user: string): Promise<void> {
  if (PROVIDER === 'rauthy') {
    // Rauthy: e-mail first, then the password field appears
    await page.locator('input[name="email"]').fill(`${user}@fathom.test`)
    await page.getByRole('button', { name: 'Login' }).click()
    const pw = page.locator('input[type="password"]:visible')
    await pw.fill(PASSWORD)
    await page.getByRole('button', { name: 'Login' }).click()
  } else {
    await page.locator('#username').fill(user)
    await page.locator('#password').fill(PASSWORD)
    await page.locator('#kc-login').click()
  }
}

/** `/` -> 302 /auth/login -> provider -> /auth/callback -> app. Returns when the app loaded. */
export async function signIn(page: Page, user: string, path = '/'): Promise<void> {
  await page.goto(path)
  await expect(page).not.toHaveURL(/^https:\/\/fathom\.localtest\.me/)
  await providerLogin(page, user)
  await page.waitForURL(
    (u) => u.hostname === 'fathom.localtest.me' && !u.pathname.startsWith('/auth/'),
  )
}

export interface ApiSession {
  status: number
  body: {
    username?: string
    groups?: string[]
    mode?: string
    refreshCount?: number
    accessModel?: string
    error?: string
  }
}

export async function apiSession(page: Page): Promise<ApiSession> {
  return page.evaluate(async () => {
    const r = await fetch('/api/session', { credentials: 'same-origin' })
    return { status: r.status, body: (await r.json()) as ApiSession['body'] }
  })
}

export interface Finished {
  ok: boolean
  error: string | null
  requestId: number | null
}

interface Frame {
  op: string
  request_id?: number
  requestId?: number
  error?: unknown
  batch?: { envelopes: Envelope[] }
}
interface Envelope {
  correlation: { type: string; data?: number }
  event: { type: string; data: { type: string; data?: unknown } & Record<string, unknown> }
}

declare global {
  interface Window {
    __raw?: { ws: WebSocket; frames: Frame[]; closed?: number | undefined }
  }
}

/** A raw `/ws` connection opened inside `page` (same origin, session cookie). */
export class RawWs {
  private readonly page: Page
  private constructor(page: Page) {
    this.page = page
  }

  static async open(page: Page): Promise<RawWs> {
    await page.evaluate(async () => {
      const ws = new WebSocket(`wss://${location.host}/ws`)
      const raw = { ws, frames: [] as Frame[], closed: undefined as number | undefined }
      window.__raw = raw
      ws.onmessage = (e: MessageEvent<string>) => raw.frames.push(JSON.parse(e.data) as Frame)
      ws.onclose = (e) => (raw.closed = e.code)
      await new Promise<void>((ok, fail) => {
        ws.onopen = () => ok()
        ws.onerror = () => fail(new Error('ws error'))
      })
      ws.send(JSON.stringify({ op: 'subscribe' }))
    })
    const raw = new RawWs(page)
    await expect.poll(() => raw.count('subscribed')).toBe(1)
    return raw
  }

  private count(op: string): Promise<number> {
    return this.page.evaluate(
      (o) => (window.__raw?.frames ?? []).filter((f) => f.op === o).length,
      op,
    )
  }

  /** Send one command; resolve with its terminal `CommandFinished` (or the bridge refusal). */
  async command(cmd: unknown, timeout = 45_000): Promise<Finished> {
    const replies = await this.page.evaluate(
      () => (window.__raw?.frames ?? []).filter((f) => f.op === 'sent' || f.op === 'error').length,
    )
    await this.page.evaluate(
      (c) => window.__raw?.ws.send(JSON.stringify({ op: 'send', cmd: c })),
      cmd,
    )
    let reply: Frame | undefined
    await expect
      .poll(
        async () => {
          reply = await this.page.evaluate(
            (n) =>
              (window.__raw?.frames ?? []).filter((f) => f.op === 'sent' || f.op === 'error')[n],
            replies,
          )
          return reply !== undefined
        },
        { timeout },
      )
      .toBe(true)
    const r = reply as Frame
    if (r.op === 'error') return { ok: false, error: JSON.stringify(r.error), requestId: null }
    const id = r.request_id ?? r.requestId ?? null
    let done: Finished | undefined
    await expect
      .poll(
        async () => {
          done = await this.page.evaluate((rid) => {
            for (const f of window.__raw?.frames ?? []) {
              for (const env of f.batch?.envelopes ?? []) {
                if (
                  env.event.type === 'CommandFinished' &&
                  env.correlation.type === 'Request' &&
                  env.correlation.data === rid
                ) {
                  const d = env.event.data as unknown as { ok: boolean; error: string | null }
                  return { ok: d.ok, error: d.error, requestId: rid }
                }
              }
            }
            return undefined
          }, id)
          return done !== undefined
        },
        { timeout },
      )
      .toBe(true)
    return done as Finished
  }

  /** Every event of kind `outer` (`Stream`, `System`, ...) with inner type `inner`. */
  events<T = unknown>(outer: string, inner: string): Promise<T[]> {
    return this.page.evaluate(
      ([o, i]) =>
        (window.__raw?.frames ?? [])
          .flatMap((f) => f.batch?.envelopes ?? [])
          .filter((e) => e.event.type === o && e.event.data.type === i)
          .map((e) => e.event.data.data as T),
      [outer, inner] as const,
    )
  }

  closeCode(): Promise<number | undefined> {
    return this.page.evaluate(() => window.__raw?.closed)
  }
}

export const b64 = (s: string): string => Buffer.from(s, 'utf8').toString('base64')
export const unb64 = (s: string): string => Buffer.from(s, 'base64').toString('utf8')

/** Name of the running `rbac-probe` pod in `demo` (exec / port-forward target). */
export function probePod(): string {
  return kubectl(
    '-n',
    'demo',
    'get',
    'pod',
    '-l',
    'app=rbac-probe',
    '--field-selector=status.phase=Running',
    '-o',
    'jsonpath={.items[0].metadata.name}',
  ).trim()
}

/**
 * Connect to 127.0.0.1:`port` from INSIDE the fathom-server pod (an ephemeral debug container
 * shares its network namespace; the port-forward listener binds there) and return what it read.
 */
export function connectFromServerPod(port: number): string {
  const pod = kubectl(
    '-n',
    'fathom',
    'get',
    'pod',
    '-l',
    'app.kubernetes.io/name=fathom',
    '-o',
    'jsonpath={.items[0].metadata.name}',
  ).trim()
  const name = `pf-${port}-${Date.now().toString(36)}`
  kubectl(
    '-n',
    'fathom',
    'debug',
    pod,
    '--image=busybox:1.37',
    '--profile=general',
    `--container=${name}`,
    '--',
    'sh',
    '-c',
    `sleep 4 | nc -w 6 127.0.0.1 ${port}; echo "nc-exit=$?"`,
  )
  for (let i = 0; i < 60; i++) {
    const state = kubectl(
      '-n',
      'fathom',
      'get',
      'pod',
      pod,
      '-o',
      `jsonpath={.status.ephemeralContainerStatuses[?(@.name=="${name}")].state.terminated.reason}`,
    ).trim()
    if (state) return kubectl('-n', 'fathom', 'logs', pod, '-c', name)
    execFileSync('sleep', ['1'])
  }
  throw new Error(`debug container ${name} did not finish`)
}
