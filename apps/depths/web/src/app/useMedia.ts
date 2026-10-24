import { useSyncExternalStore } from 'react'

/** Whether `query` matches, re-rendering when it flips. `true` where there is no `matchMedia`. */
export function useMedia(query: string): boolean {
  return useSyncExternalStore(
    (onChange) => {
      const q = typeof globalThis.matchMedia === 'function' ? globalThis.matchMedia(query) : null
      q?.addEventListener('change', onChange)
      return () => q?.removeEventListener('change', onChange)
    },
    () => (typeof globalThis.matchMedia === 'function' ? globalThis.matchMedia(query).matches : true),
  )
}
