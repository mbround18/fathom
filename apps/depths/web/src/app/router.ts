/**
 * A small history router: the app has a handful of flat routes, so a path matcher over
 * `history.pushState` is all it needs. Patterns are `/literal/:param`; params are decoded.
 */
import { useSyncExternalStore } from 'react'

const NAVIGATE = 'depths:navigate'

function subscribe(onChange: () => void): () => void {
  globalThis.addEventListener('popstate', onChange)
  globalThis.addEventListener(NAVIGATE, onChange)
  return () => {
    globalThis.removeEventListener('popstate', onChange)
    globalThis.removeEventListener(NAVIGATE, onChange)
  }
}

const currentPath = (): string => globalThis.location.pathname

/** The current pathname, re-rendering on navigation. */
export function usePath(): string {
  return useSyncExternalStore(subscribe, currentPath, () => '/')
}

/** Go to a same-origin path. Anything that is not a plain absolute path is ignored. */
export function navigate(to: string, opts: { replace?: boolean } = {}): void {
  if (!to.startsWith('/') || to.startsWith('//')) return
  if (opts.replace) globalThis.history.replaceState(null, '', to)
  else globalThis.history.pushState(null, '', to)
  globalThis.dispatchEvent(new Event(NAVIGATE))
}

export type Params = Readonly<Record<string, string>>

/** Match `path` against `pattern`; `null` when it does not match. Trailing slashes are ignored. */
export function matchPath(pattern: string, path: string): Params | null {
  const want = pattern.split('/').filter(Boolean)
  const got = path.split('/').filter(Boolean)
  if (want.length !== got.length) return null
  const params: Record<string, string> = {}
  for (const [i, seg] of want.entries()) {
    const value = got[i] ?? ''
    if (seg.startsWith(':')) {
      try {
        params[seg.slice(1)] = decodeURIComponent(value)
      } catch {
        return null
      }
    } else if (seg !== value) {
      return null
    }
  }
  return params
}
