import { fireEvent, render, screen } from '@testing-library/react'
import { act } from 'react'
import { beforeEach, describe, expect, it } from 'vitest'

import GrafanaPanel from '@/components/panels/GrafanaPanel'
import { PrometheusForm } from '@/features/metrics/PrometheusForm'
import type { BackendCmd, SystemEvent } from '@/protocol/types'
import { useStore } from '@/store'
import { ingest } from '@/store/ingest'

import { batch, system } from '../fixtures/builders'

let sent: BackendCmd[] = []
let seq = 100

function settings(
  context: string,
  grafana: { base_url: string; key_set: boolean } | null,
  prometheus: { namespace: string; service: string; port: number } | null = null,
) {
  const ev: SystemEvent = { type: 'ObservabilitySettings', data: { context, prometheus, grafana } }
  act(() => ingest(batch([system(seq++, ev)])))
}

beforeEach(() => {
  sent = []
  useStore.setState({
    send: (c: BackendCmd) => void sent.push(c),
    integrations: { portAvailability: {} },
    session: { ...useStore.getState().session, activeContext: 'kind-a' },
  } as never)
})

describe('ObservabilitySettings handler', () => {
  it('stores by context and ignores other contexts', () => {
    settings('kind-b', { base_url: 'https://b', key_set: true })
    expect(useStore.getState().integrations.settings).toBeUndefined()
    settings('kind-a', { base_url: 'https://a', key_set: false })
    expect(useStore.getState().integrations.settings?.['kind-a']?.grafana?.base_url).toBe(
      'https://a',
    )
  })

  it('falls back to the default context when none is active', () => {
    useStore.setState({ session: { ...useStore.getState().session, activeContext: undefined } })
    settings('default', { base_url: 'https://d', key_set: true })
    expect(useStore.getState().integrations.settings?.default?.grafana?.key_set).toBe(true)
  })
})

describe('GrafanaPanel hydration', () => {
  it('requests settings on open and prefills URL but never the key', () => {
    render(<GrafanaPanel />)
    expect(sent.some((c) => c.type === 'GetObservabilitySettings')).toBe(true)
    expect(screen.getByTestId('grafana-configured').textContent).toContain('Not configured')
    settings('kind-a', { base_url: 'https://g.example', key_set: true })
    expect(screen.getByLabelText('Base URL')).toHaveProperty('value', 'https://g.example')
    expect(screen.getByLabelText('API key')).toHaveProperty('value', '')
    expect(screen.getByTestId('grafana-configured').textContent).toContain('Configured')
    expect(screen.getByTestId('grafana-key-state').textContent).toBe('Key set')
  })

  it('blank key on save sends an empty key (backend keeps the stored one)', () => {
    render(<GrafanaPanel />)
    settings('kind-a', { base_url: 'https://g.example', key_set: true })
    fireEvent.click(screen.getByText('Save'))
    expect(sent.at(-1)).toEqual({
      type: 'ConfigureGrafana',
      data: { base_url: 'https://g.example', api_key: '' },
    })
  })

  it('Clear key sends ClearGrafanaKey and is disabled without a key', () => {
    render(<GrafanaPanel />)
    expect(screen.getByText('Clear key')).toHaveProperty('disabled', true)
    settings('kind-a', { base_url: 'https://g.example', key_set: true })
    fireEvent.click(screen.getByText('Clear key'))
    expect(sent.at(-1)).toEqual({ type: 'ClearGrafanaKey' })
    settings('kind-a', { base_url: 'https://g.example', key_set: false })
    expect(screen.getByTestId('grafana-key-state').textContent).toBe('Key not set')
  })

  it('re-requests settings when the context changes', () => {
    render(<GrafanaPanel />)
    sent = []
    act(() =>
      useStore.setState({ session: { ...useStore.getState().session, activeContext: 'kind-b' } }),
    )
    expect(sent.some((c) => c.type === 'GetObservabilitySettings')).toBe(true)
  })
})

describe('PrometheusForm hydration', () => {
  it('shows defaults as not configured, then prefills saved values', () => {
    render(<PrometheusForm />)
    expect(screen.getByTestId('prometheus-configured').textContent).toContain('Not configured')
    expect(screen.getByLabelText('Namespace')).toHaveProperty('value', 'monitoring')
    settings('kind-a', null, { namespace: 'obs', service: 'prom', port: 9091 })
    expect(screen.getByLabelText('Namespace')).toHaveProperty('value', 'obs')
    expect(screen.getByLabelText('Service')).toHaveProperty('value', 'prom')
    expect(screen.getByLabelText('Port')).toHaveProperty('value', '9091')
    expect(screen.getByTestId('prometheus-configured').textContent).toBe('Configured')
  })
})
