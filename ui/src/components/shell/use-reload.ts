import { useEffect, useState } from 'react'

import { useStore } from '@/store'

/** How long "Reload kubeconfig" waits for the backend's answer before saying it got none. */
export const RELOAD_TIMEOUT_MS = 15_000

/**
 * "Reload kubeconfig" state (feedback #1): pending from the click until the next
 * `KubeconfigStatus` arrives (`kubeconfigLoads` moves), then the outcome, so an unchanged
 * result still gets an answer.
 */
export function useReload(send: (cmd: { type: 'Initialize' }) => void) {
  const loads = useStore((s) => s.session.kubeconfigLoads)
  const [from, setFrom] = useState<number | undefined>(undefined)
  const [timedOut, setTimedOut] = useState(false)
  const done = from !== undefined && loads > from
  const pending = from !== undefined && !done && !timedOut
  useEffect(() => {
    if (!pending) return
    const t = setTimeout(() => setTimedOut(true), RELOAD_TIMEOUT_MS)
    return () => clearTimeout(t)
  }, [pending])
  return {
    pending,
    done,
    timedOut: timedOut && !done,
    reload: () => {
      setFrom(loads)
      setTimedOut(false)
      send({ type: 'Initialize' })
    },
  }
}
