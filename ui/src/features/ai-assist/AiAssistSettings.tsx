import { useEffect, useState } from 'react'

import { Button } from '@/components/ui/button'
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

  const [confirmFile, setConfirmFile] = useState(false)
  const keyringDown = ai.statuses.some(
    (s) => s.state.type === 'Blocked' && s.state.data.reason === 'store_unavailable',
  )
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
      {keyringDown && (
        <div
          data-testid="ai-file-store"
          className="flex flex-col gap-2 border border-border p-2 text-xs"
        >
          <p role="alert" className="text-status-warn">
            No OS credential store is available. You can opt in to storing provider keys in a local
            file (permissions 0600). This is less secure than the OS keychain: anyone with access to
            your user account can read it. Keys are never displayed.
          </p>
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              data-testid="ai-file-store-confirm"
              checked={confirmFile}
              onChange={(e) => setConfirmFile(e.target.checked)}
            />
            I understand and want file-based credential storage
          </label>
          <div>
            <Button
              size="sm"
              variant="outline"
              data-testid="ai-file-store-enable"
              disabled={!confirmFile}
              onClick={() => {
                void request({ type: 'AiEnableFileStore', data: { confirm: true } })
                setConfirmFile(false)
              }}
            >
              Enable file store
            </Button>
          </div>
        </div>
      )}
      <OllamaPicker />
    </fieldset>
  )
}
