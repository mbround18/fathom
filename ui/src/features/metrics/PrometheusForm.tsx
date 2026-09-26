import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useStore } from '@/store'

import { useObservabilitySettings } from './useObservabilitySettings'

/** Configure + test the in-cluster Prometheus service; also shows the stack scan result. */
export function PrometheusForm() {
  const send = useStore((s) => s.send)
  const test = useStore((s) => s.integrations.prometheusTest)
  const scan = useStore((s) => s.metrics.stackScan)
  const [namespace, setNamespace] = useState('monitoring')
  const [service, setService] = useState('prometheus')
  const [port, setPort] = useState('9090')
  const settings = useObservabilitySettings()
  const saved = settings?.prometheus ?? null
  const [hydratedFrom, setHydratedFrom] = useState<typeof settings>(undefined)
  if (settings !== undefined && settings !== hydratedFrom) {
    setHydratedFrom(settings)
    if (settings.prometheus) {
      setNamespace(settings.prometheus.namespace)
      setService(settings.prometheus.service)
      setPort(String(settings.prometheus.port))
    }
  }
  const portNum = Number(port)
  const valid =
    namespace.trim() !== '' &&
    service.trim() !== '' &&
    Number.isInteger(portNum) &&
    portNum > 0 &&
    portNum < 65536
  const data = { namespace: namespace.trim(), service: service.trim(), port: portNum }
  const suggested = scan?.suggested_endpoint

  return (
    <section aria-label="Prometheus connection" className="flex flex-col gap-2 p-3">
      <h3 className="text-sm font-medium">Prometheus</h3>
      <p data-testid="prometheus-configured" className="text-xs text-muted-foreground">
        {saved ? 'Configured' : 'Not configured (showing defaults)'}
      </p>
      <form
        className="grid grid-cols-1 gap-2 sm:grid-cols-3"
        onSubmit={(e) => {
          e.preventDefault()
          if (valid) send({ type: 'ConfigurePrometheus', data })
        }}
      >
        <div className="flex flex-col gap-1 text-xs">
          <span>Namespace</span>
          <Input
            aria-label="Namespace"
            value={namespace}
            onChange={(e) => setNamespace(e.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1 text-xs">
          <span>Service</span>
          <Input
            aria-label="Service"
            value={service}
            onChange={(e) => setService(e.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1 text-xs">
          <span>Port</span>
          <Input
            aria-label="Port"
            inputMode="numeric"
            value={port}
            onChange={(e) => setPort(e.target.value)}
            aria-invalid={!valid}
          />
        </div>
        <div className="flex flex-wrap gap-2 sm:col-span-3">
          <Button
            type="button"
            variant="outline"
            disabled={!valid}
            onClick={() => send({ type: 'TestPrometheusConnection', data })}
          >
            Test connection
          </Button>
          <Button type="submit" disabled={!valid}>
            Save
          </Button>
          <Button
            type="button"
            variant="ghost"
            onClick={() => send({ type: 'RefreshIntegrations' })}
          >
            Scan cluster
          </Button>
        </div>
      </form>
      {test && (
        <p
          role="status"
          data-testid="prometheus-test-result"
          className={test.success ? 'text-xs' : 'text-xs text-destructive'}
        >
          {test.success ? 'Connected: ' : 'Failed: '}
          {test.message}
        </p>
      )}
      {scan && (
        <div data-testid="stack-scan" className="text-xs">
          <p>Metrics stack: {scan.tier}</p>
          <ul className="list-disc pl-4">
            {scan.components.map((c) => (
              <li key={`${c.kind}/${c.namespace}/${c.service}`}>
                {c.kind} {c.namespace}/{c.service}:{c.port}
              </li>
            ))}
          </ul>
          {suggested && (
            <Button
              variant="link"
              className="h-auto p-0 text-xs"
              onClick={() => {
                setNamespace(suggested.namespace)
                setService(suggested.service)
                setPort(String(suggested.port))
              }}
            >
              Use {suggested.namespace}/{suggested.service}
            </Button>
          )}
        </div>
      )}
    </section>
  )
}
