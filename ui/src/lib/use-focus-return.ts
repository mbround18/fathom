import { useCallback, useLayoutEffect, useRef } from 'react'

/**
 * Restores focus to whatever held it when a controlled dialog/sheet opened. Radix only does this
 * on its own for `<Trigger>`-driven overlays; ours open from store state, and focus otherwise
 * falls to `<body>`. Pass the returned handler as `onCloseAutoFocus`.
 */
export function useFocusReturn(open: boolean): (event: Event) => void {
  const opener = useRef<HTMLElement | null>(null)
  useLayoutEffect(() => {
    if (!open) return
    const active = document.activeElement
    opener.current = active instanceof HTMLElement && active !== document.body ? active : null
  }, [open])
  return useCallback((event: Event) => {
    const el = opener.current
    if (el?.isConnected) {
      event.preventDefault()
      el.focus()
    }
  }, [])
}
