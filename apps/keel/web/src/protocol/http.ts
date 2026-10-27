/**
 * The HTTP side of sign-in (apps/keel/server/src/signin.rs).
 *
 * Keel has no `/api/v1/me`: the page learns it has a session from the readable CSRF cookie the
 * server sets beside the HttpOnly session cookie (`__Host-keel_csrf`, or `keel_csrf` under
 * `--insecure-dev` on plain HTTP), then asks `GetMe` over the socket. The token is read from the
 * cookie, kept in memory, and sent in `Hello` and as `X-CSRF-Token`; never written anywhere else.
 */

/** The CSRF cookie names, secure first. */
export const CSRF_COOKIES = ['__Host-keel_csrf', 'keel_csrf'] as const

/** The session's CSRF token from a `document.cookie` string, or `null` without one. */
export function readCsrf(cookies: string): string | null {
  const jar = new Map<string, string>()
  for (const part of cookies.split(';')) {
    const eq = part.indexOf('=')
    if (eq < 0) continue
    const name = part.slice(0, eq).trim()
    const value = part.slice(eq + 1).trim()
    if (!jar.has(name)) jar.set(name, value)
  }
  for (const name of CSRF_COOKIES) {
    const v = jar.get(name)
    // The server's token is URL-safe base64; anything else is not ours and is ignored.
    if (v && /^[A-Za-z0-9_-]{1,128}$/.test(v)) return v
  }
  return null
}

/**
 * Where "Sign in" goes. The server picks the provider when one is configured and redirects to
 * it; the provider's callback lands on `/`.
 */
export const LOGIN_HREF = '/auth/login'

export type FetchLike = (input: string, init?: RequestInit) => Promise<Response>

const defaultFetch: FetchLike = (input, init) => globalThis.fetch(input, init)

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

/** `endSessionUrl` from a logout reply, when it is an http(s) URL; anything else is ignored. */
export async function endSessionUrl(res: Response): Promise<string | null> {
  const body: unknown = await res.json().catch(() => null)
  if (body && typeof body === 'object' && 'endSessionUrl' in body) {
    const u = body.endSessionUrl
    if (typeof u === 'string' && /^https?:\/\//.test(u)) return u
  }
  return null
}
