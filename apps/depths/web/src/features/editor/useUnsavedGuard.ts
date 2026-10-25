/**
 * While `dirty`, leaving the page asks first: in-app navigation and Back/Forward through the
 * router's guard, closing or reloading the tab through `beforeunload`. `release()` lets one
 * navigation through without asking (after the save it was waiting for has started).
 */
import { useCallback, useEffect, useRef } from 'react'

import { setNavigationGuard } from '@/app/router'

export const UNSAVED_PROMPT = 'You have unsaved changes. Leave and discard them?'

export function useUnsavedGuard(dirty: boolean): { release: () => void } {
  const released = useRef(false)

  useEffect(() => {
    if (!dirty) return
    released.current = false
    setNavigationGuard(() => released.current || globalThis.confirm(UNSAVED_PROMPT))
    const onUnload = (e: BeforeUnloadEvent): void => {
      if (released.current) return
      e.preventDefault()
    }
    globalThis.addEventListener('beforeunload', onUnload)
    return () => {
      setNavigationGuard(null)
      globalThis.removeEventListener('beforeunload', onUnload)
    }
  }, [dirty])

  return {
    release: useCallback(() => {
      released.current = true
    }, []),
  }
}
