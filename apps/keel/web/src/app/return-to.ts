/**
 * The sign-in redirect's way back. The server's callback always lands on `/`, so the path a
 * signed-out visitor asked for is kept in `sessionStorage` (this tab only) when they press
 * "Sign in", and restored once `Me` arrives. Only a plain same-origin path is ever stored or used.
 */
const KEY = 'keel:return-to'

function safePath(p: string | null): string | null {
  if (!p?.startsWith('/') || p.startsWith('//') || p.startsWith('/auth/') || p.startsWith('/api/')) return null
  return p
}

export function rememberReturnTo(path: string): void {
  try {
    const p = safePath(path)
    if (p && p !== '/') globalThis.sessionStorage.setItem(KEY, p)
    else globalThis.sessionStorage.removeItem(KEY)
  } catch {
    /* Storage blocked: the visitor lands on the home page instead. */
  }
}

/** The remembered path, once; `null` when there is none. */
export function takeReturnTo(): string | null {
  try {
    const p = globalThis.sessionStorage.getItem(KEY)
    globalThis.sessionStorage.removeItem(KEY)
    return safePath(p)
  } catch {
    return null
  }
}
