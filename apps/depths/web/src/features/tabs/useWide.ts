import { useSyncExternalStore } from 'react'

/** Side-by-side panes from this width up; below it one pane with a tab strip (FR-024). */
const WIDE = '(min-width: 768px)'

function query(): MediaQueryList | null {
  return typeof globalThis.matchMedia === 'function' ? globalThis.matchMedia(WIDE) : null
}

function subscribe(onChange: () => void): () => void {
  const q = query()
  q?.addEventListener('change', onChange)
  return () => q?.removeEventListener('change', onChange)
}

export function useWide(): boolean {
  return useSyncExternalStore(subscribe, () => query()?.matches ?? true)
}
