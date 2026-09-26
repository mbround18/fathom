import { useEffect, useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useObservabilitySettings } from '@/features/metrics/useObservabilitySettings'
import { useStore } from '@/store'

/** Grafana configuration. The API key is write-only: it is never rendered back or logged. */
export default function GrafanaPanel() {
  const send = useStore((s) => s.send)
  const test = useStore((s) => s.integrations.grafanaTest)
  const scan = useStore((s) => s.metrics.stackScan)
  const context = useStore((s) => s.session.activeContext)
  const detected = scan?.components.filter((c) => c.kind === 'Grafana') ?? []
  // Look for an in-cluster Grafana once per context so the pane can say where it is.
  useEffect(() => {
    if (context !== undefined && scan === undefined) send({ type: 'RefreshIntegrations' })
  }, [context, scan, send])
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
      <div data-testid="grafana-detected" className="text-xs text-muted-foreground">
        {scan === undefined ? (
          'Scanning the cluster for Grafana…'
        ) : detected.length > 0 ? (
          <ul className="flex flex-col gap-1">
            {detected.map((c) => (
              <li key={`${c.namespace}/${c.service}`} className="flex flex-wrap items-center gap-2">
                <span>
                  In cluster:{' '}
                  <span className="font-mono">{`${c.namespace}/${c.service}:${c.port}`}</span>.
                  Forward it (
                  <span className="font-mono">{`kubectl -n ${c.namespace} port-forward svc/${c.service} ${c.port}`}</span>
                  ), then use its local URL.
                </span>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => setUrl(`http://localhost:${c.port}`)}
                >
                  Use http://localhost:{c.port}
                </Button>
              </li>
            ))}
          </ul>
        ) : (
          <span>
            No Grafana service found in this cluster. Enter the URL of an external Grafana, or
            install one (dev clusters: <span className="font-mono">k8s/addons/grafana</span>).
          </span>
        )}
      </div>
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
