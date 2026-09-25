import { useEffect } from 'react'

import { Switch } from '@/components/ui/switch'
import { useStore } from '@/store'

import { OllamaPicker } from './OllamaPicker'
import { ProviderRow } from './ProviderLogin'

/** Settings -> AI Assist (038). Everything shown comes from backend `Ai*` events. */
export function AiAssistSettings() {
  const request = useStore((s) => s.request)
  const ai = useStore((s) => s.ai)

  useEffect(() => {
    void request({ type: 'AiGetStatus' })
    void request({ type: 'AiOllamaProbe', data: { endpoint: null } })
  }, [request])

  const hosted = ai.statuses.filter((s) => s.provider !== 'ollama')
  return (
    <fieldset data-testid="ai-assist-settings" className="flex flex-col gap-3">
      <legend className="text-xs font-semibold tracking-wider uppercase">AI Assist</legend>

      {ai.migration?.type === 'Migrated' && (
        <p data-testid="ai-migration-notice" className="text-xs">
          Your saved API key moved to the OS keychain.
        </p>
      )}
      {ai.migration?.type === 'Failed' && (
        <p data-testid="ai-migration-failed" className="text-xs text-destructive">
          Could not move your saved key: {ai.migration.data.message}. It was left in place.
        </p>
      )}

      <div className="flex items-center justify-between gap-2 text-xs">
        <span>Offline mode — Ollama only, no outbound requests</span>
        <Switch
          data-testid="ai-offline-toggle"
          aria-label="Offline mode"
          checked={ai.offline}
          onCheckedChange={(enabled) =>
            void request({ type: 'AiSetOfflineMode', data: { enabled } })
          }
        />
      </div>

      {ai.lastError !== undefined && (
        <p data-testid="ai-error" role="alert" className="text-xs text-destructive">
          {ai.lastError.message}
          {ai.lastError.hint !== '' && (
            <span className="block text-muted-foreground">{ai.lastError.hint}</span>
          )}
        </p>
      )}

      <ul className="flex flex-col">
        {hosted.map((s) => (
          <ProviderRow key={s.provider} status={s} />
        ))}
      </ul>
      <OllamaPicker />
    </fieldset>
  )
}
