import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useStore } from '@/store'

/** Grafana configuration. The API key is write-only: it is never rendered back or logged. */
export default function GrafanaPanel() {
  const send = useStore((s) => s.send)
  const test = useStore((s) => s.integrations.grafanaTest)
  const [url, setUrl] = useState('')
  const [key, setKey] = useState('')
  const validUrl = /^https?:\/\/\S+$/.test(url.trim())
  const data = () => ({ base_url: url.trim(), api_key: key })
  return (
    <section aria-label="Grafana" className="flex max-w-xl flex-col gap-2 p-3">
      <h3 className="text-sm font-medium">Grafana</h3>
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
            onChange={(e) => setKey(e.target.value)}
          />
        </div>
        <div className="flex gap-2">
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
