import { useEffect, useState } from 'react'
import { toast } from 'sonner'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useObservabilitySettings } from '@/features/metrics/useObservabilitySettings'
import { SECTION } from '@/protocol/commands'
import type { DetectedMetricsComponent, PortForward } from '@/protocol/types'
import { listKey, serviceForwardKey, useStore } from '@/store'
import { waitForCommandResult } from '@/store/actions'

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
              <GrafanaConnect
                key={`${c.namespace}/${c.service}`}
                component={c}
                savedUrl={saved?.base_url}
                onUrl={setUrl}
              />
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

/**
 * 041: one-click access to an in-cluster Grafana. Starts a Fathom port-forward to the Service
 * (listed under Port forwards like any other), saves `http://localhost:<port>` as the base URL
 * and opens it. Grafana refuses to be framed by default, so it opens in the system browser.
 */
function GrafanaConnect({
  component: c,
  savedUrl,
  onUrl,
}: {
  component: DetectedMetricsComponent
  savedUrl: string | undefined
  onUrl: (url: string) => void
}) {
  const send = useStore((s) => s.send)
  const request = useStore((s) => s.request)
  const key = serviceForwardKey(c.namespace, c.service, c.port)
  const forward = useStore((s) => s.integrations.serviceForwards?.[key])
  // The forward is still listed (it can be stopped or removed from Port forwards).
  const tunnelUp = useStore((s) => {
    const rows = s.lists[listKey(null, SECTION.portForwards, 'Global', null)]?.rows as
      PortForward[] | undefined
    return forward !== undefined && (rows ?? []).some((pf) => pf.id === forward.tunnelId)
  })
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  const localUrl = forward && tunnelUp ? `http://localhost:${String(forward.localPort)}` : undefined

  const connect = () => {
    setBusy(true)
    setError(undefined)
    void (async () => {
      try {
        const id = await request({
          type: 'StartServicePortForward',
          data: { namespace: c.namespace, service: c.service, port: c.port },
        })
        const result = await waitForCommandResult(id)
        if (result === 'timeout' || !result.ok) {
          setError(result === 'timeout' ? 'No response from backend.' : (result.error ?? 'Failed'))
          return
        }
        const ready = useStore.getState().integrations.serviceForwards?.[key]
        if (!ready) {
          setError('The forward started but its local port is unknown; see Port forwards.')
          return
        }
        const url = `http://localhost:${String(ready.localPort)}`
        onUrl(url)
        // Save the local URL (a blank key keeps any saved API key) so Grafana links work.
        if (savedUrl !== url)
          send({ type: 'ConfigureGrafana', data: { base_url: url, api_key: '' } })
        toast.success(`Grafana forwarded to ${url}`)
        window.open(url, '_blank', 'noopener,noreferrer')
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setBusy(false)
      }
    })()
  }

  return (
    <li data-testid="grafana-detected-item" className="flex flex-col gap-1">
      <span>
        In cluster:{' '}
        <span className="font-mono">{`${c.namespace}/${c.service}:${String(c.port)}`}</span>
      </span>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          size="sm"
          data-testid="grafana-connect"
          disabled={busy}
          onClick={connect}
        >
          {busy ? 'Connecting…' : localUrl ? 'Reopen Grafana' : 'Open Grafana'}
        </Button>
        {localUrl && (
          <a
            data-testid="grafana-local-url"
            className="font-mono underline"
            href={localUrl}
            target="_blank"
            rel="noreferrer"
          >
            {localUrl}
          </a>
        )}
      </div>
      {error && (
        <span role="alert" data-testid="grafana-connect-error" className="text-destructive">
          {error}
        </span>
      )}
    </li>
  )
}
