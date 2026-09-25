import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import GrafanaPanel from '@/components/panels/GrafanaPanel'
import MetricsPanel from '@/components/panels/MetricsPanel'
import type { BackendCmd } from '@/protocol/types'
import { useStore } from '@/store'

let sent: BackendCmd[] = []

beforeEach(() => {
  sent = []
  useStore.setState({
    send: (c: BackendCmd) => void sent.push(c),
    integrations: { portAvailability: {} },
    metrics: { diskUsage: [] },
    details: {},
    session: { ...useStore.getState().session, activeContext: 'kind' },
  } as never)
})

describe('MetricsPanel', () => {
  it('shows unavailable state with no data and polls start/stop with mount', () => {
    const { unmount } = render(<MetricsPanel />)
    expect(screen.getByText(/No metrics received yet/)).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Pod namespace'), { target: { value: 'default' } })
    fireEvent.change(screen.getByLabelText('Pod name'), { target: { value: 'web' } })
    fireEvent.click(screen.getByText('Start polling'))
    expect(sent.at(-1)?.type).toBe('StartMetricsPolling')
    unmount()
    expect(sent.at(-1)?.type).toBe('StopMetricsPolling')
  })

  it('renders pod metrics and prometheus test result', () => {
    useStore.setState({
      integrations: { portAvailability: {}, prometheusTest: { success: true, message: 'ok v2' } },
      details: {
        k: {
          kind: 'PrometheusMetrics',
          eventType: 'PrometheusMetricsUpdated',
          updatedAt: 1,
          data: {
            namespace: 'default',
            name: 'web',
            cpu: { usage_percent: 12.5, cores: 0.1 },
            memory: { usage_bytes: 2048, usage_percent: 5, limit_bytes: null },
            network: { bytes_in: null, bytes_out: null, packets_in: null, packets_out: null },
            timestamp: 1,
          },
        },
      },
    } as never)
    render(<MetricsPanel />)
    expect(screen.getByTestId('pod-metrics-row').textContent).toContain('12.5%')
    expect(screen.getByTestId('metrics-status').textContent).toContain('connected')
    expect(screen.getByTestId('prometheus-test-result').textContent).toContain('ok v2')
  })

  it('sends TestPrometheusConnection with a numeric port', () => {
    render(<MetricsPanel />)
    fireEvent.click(screen.getByText('Test connection'))
    expect(sent.at(-1)).toEqual({
      type: 'TestPrometheusConnection',
      data: { namespace: 'monitoring', service: 'prometheus', port: 9090 },
    })
  })
})

describe('GrafanaPanel', () => {
  it('validates URL, sends config, never echoes the key', () => {
    const spy = vi.spyOn(console, 'log')
    render(<GrafanaPanel />)
    expect(screen.getByText<HTMLButtonElement>('Save').disabled).toBe(true)
    fireEvent.change(screen.getByLabelText('Base URL'), { target: { value: 'https://g.example' } })
    fireEvent.change(screen.getByLabelText('API key'), { target: { value: 'sekret' } })
    fireEvent.click(screen.getByText('Test connection'))
    expect(sent.at(-1)).toEqual({
      type: 'TestGrafanaConnection',
      data: { base_url: 'https://g.example', api_key: 'sekret' },
    })
    expect(screen.getByLabelText<HTMLInputElement>('API key').type).toBe('password')
    fireEvent.click(screen.getByText('Save'))
    expect(screen.getByLabelText<HTMLInputElement>('API key').value).toBe('')
    expect(spy).not.toHaveBeenCalled()
  })
})
