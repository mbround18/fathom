/**
 * The HTTP side of the contract (contracts/protocol.md, "HTTP endpoints").
 *
 * `GET /api/v1/me` bootstraps the page. Its `csrf_token` is kept in memory by the session store
 * and sent as `X-CSRF-Token` on every mutating request; it is never written to storage or a URL.
 */
import type { Me, Provider } from './generated'

const PROVIDERS: readonly Provider[] = ['oidc', 'github', 'discord']

export type MeResult =
  | { kind: 'ok'; me: Me }
  /**
   * 401. `providers` is what the server's 401 body lists as configured; empty when it lists
   * none, in which case the sign-in page offers the plain `/auth/login`.
   */
  | { kind: 'unauthenticated'; providers: Provider[] }
  | { kind: 'error'; status: number }

export type FetchLike = (input: string, init?: RequestInit) => Promise<Response>

const defaultFetch: FetchLike = (input, init) => globalThis.fetch(input, init)

export async function fetchMe(fetchFn: FetchLike = defaultFetch): Promise<MeResult> {
  let res: Response
  try {
    res = await fetchFn('/api/v1/me', {
      credentials: 'same-origin',
      headers: { Accept: 'application/json' },
    })
  } catch {
    return { kind: 'error', status: 0 }
  }
  if (res.status === 401) return { kind: 'unauthenticated', providers: await providersOf(res) }
  if (!res.ok) return { kind: 'error', status: res.status }
  try {
    return { kind: 'ok', me: (await res.json()) as Me }
  } catch {
    return { kind: 'error', status: res.status }
  }
}

/** Only names this build knows: anything else in the body is ignored, never rendered. */
async function providersOf(res: Response): Promise<Provider[]> {
  try {
    const body: unknown = await res.json()
    if (typeof body !== 'object' || body === null || !('providers' in body)) return []
    const list: unknown = body.providers
    if (!Array.isArray(list)) return []
    return PROVIDERS.filter((p) => list.includes(p))
  } catch {
    return []
  }
}

/** Where a provider's button goes. A fixed path with a known provider name; nothing else. */
export function loginHref(provider?: Provider): string {
  return provider ? `/auth/login?provider=${provider}` : '/auth/login'
}

/** A same-origin mutating request carrying the session's CSRF token. */
export function mutate(
  path: string,
  csrfToken: string,
  init: Omit<RequestInit, 'credentials'> = {},
  fetchFn: FetchLike = defaultFetch,
): Promise<Response> {
  const headers = new Headers(init.headers)
  headers.set('X-CSRF-Token', csrfToken)
  return fetchFn(path, { method: 'POST', ...init, headers, credentials: 'same-origin' })
}
