import { useEffect } from 'react'

import { useStore, type ObservabilitySettingsEntry } from '@/store'

/**
 * 039: saved observability settings for the active context. Requests them on mount and whenever
 * the context changes (the backend falls back to the "default" context when none is active).
 * Returns `undefined` until the backend has answered for this context.
 */
export function useObservabilitySettings(): ObservabilitySettingsEntry | undefined {
  const send = useStore((s) => s.send)
  const context = useStore((s) => s.session.activeContext) ?? 'default'
  const settings = useStore((s) => s.integrations.settings?.[context])
  useEffect(() => {
    send({ type: 'GetObservabilitySettings' })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [context])
  return settings
}
