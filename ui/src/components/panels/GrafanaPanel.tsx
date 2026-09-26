import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useObservabilitySettings } from '@/features/metrics/useObservabilitySettings'
import { useStore } from '@/store'

/** Grafana configuration. The API key is write-only: it is never rendered back or logged. */
export default function GrafanaPanel() {
  const send = useStore((s) => s.send)
  const test = useStore((s) => s.integrations.grafanaTest)
  const [url, setUrl] = useState('')
  const [key, setKey] = useState('')
  const settings = useObservabilitySettings()
  const saved = settings?.grafana ?? null
  // Adjust state during render when a fresh settings answer arrives (context switch or save):
  // prefill the URL only; the key input is never prefilled.
  const [hydratedFrom, setHydratedFrom] = useState<typeof settings>(undefined)
  if (settings !== undefined && settings !== hydratedFrom) {
    setHydratedFrom(settings)
    setUrl(settings.grafana?.base_url ?? '')
    setKey('')
  }
  const validUrl = /^https?:\/\/\S+$/.test(url.trim())
  const data = () => ({ base_url: url.trim(), api_key: key })
  return (
    <section aria-label="Grafana" className="flex max-w-xl flex-col gap-2 p-3">
      <h3 className="text-sm font-medium">Grafana</h3>
      <p data-testid="grafana-configured" className="text-xs text-muted-foreground">
        {saved ? 'Configured' : 'Not configured'}
        {' · '}
        <span data-testid="grafana-key-state">{saved?.key_set ? 'Key set' : 'Key not set'}</span>
      </p>
      <form
        className="flex flex-col gap-2"
        autoComplete="off"
        onSubmit={(e) => {
          e.preventDefault()
          if (validUrl) {
            send({ type: 'ConfigureGrafana', data: data() })
            setKey('')
          }
        }}
      >
        <div className="flex flex-col gap-1 text-xs">
          <span>Base URL</span>
          <Input
            aria-label="Base URL"
            value={url}
            placeholder="https://grafana.example.com"
            onChange={(e) => setUrl(e.target.value)}
            aria-invalid={url !== '' && !validUrl}
          />
        </div>
        <div className="flex flex-col gap-1 text-xs">
          <span>API key</span>
          <Input
            aria-label="API key"
            type="password"
            autoComplete="off"
            value={key}
            placeholder={saved?.key_set ? 'Leave blank to keep the saved key' : ''}
            onChange={(e) => setKey(e.target.value)}
          />
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            disabled={!validUrl}
            onClick={() => send({ type: 'TestGrafanaConnection', data: data() })}
          >
            Test connection
          </Button>
          <Button type="submit" disabled={!validUrl}>
            Save
          </Button>
          <Button
            type="button"
            variant="ghost"
            disabled={!saved?.key_set}
            onClick={() => send({ type: 'ClearGrafanaKey' })}
          >
            Clear key
          </Button>
        </div>
      </form>
      {test && (
        <p
          role="status"
          data-testid="grafana-test-result"
          className={test.success ? 'text-xs' : 'text-xs text-destructive'}
        >
          {test.success ? 'Connected: ' : 'Failed: '}
          {test.message}
        </p>
      )}
    </section>
  )
}
