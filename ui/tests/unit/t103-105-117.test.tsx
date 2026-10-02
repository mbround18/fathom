import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { MetricsChart } from '@/components/MetricsChart'
import { InspectorMetrics } from '@/components/InspectorMetrics'
import { AppearanceSettings } from '@/components/panels/AppearanceSettings'
import { SHORTCUTS } from '@/lib/shortcuts'
import type { BackendCmd } from '@/protocol/types'
import { SAVE_DEBOUNCE_MS, useStore } from '@/store'

let sent: BackendCmd[] = []

beforeEach(() => {
  sent = []
  useStore.setState({
    send: (c: BackendCmd) => void sent.push(c),
    details: {},
  } as never)
})
afterEach(() => vi.useRealTimers())

function podDetail(ts: number) {
  return {
    kind: 'PrometheusMetrics',
    eventType: 'PrometheusMetricsUpdated',
    updatedAt: ts,
    data: {
      namespace: 'default',
      name: 'web',
      cpu: { usage_percent: 12.5, cores: 0.1 },
      memory: { usage_bytes: 2048, usage_percent: 5, limit_bytes: null },
      network: { bytes_in: null, bytes_out: null, packets_in: null, packets_out: null },
      timestamp: ts,
    },
  }
}

describe('InspectorMetrics (T105)', () => {
  it('says node metrics are unavailable', () => {
    render(<InspectorMetrics target={{ kind: 'Node', name: 'n1' }} />)
    expect(screen.getByText(/No node metrics yet/)).toBeTruthy()
  })

  it('shows unavailable for a pod with no metrics', () => {
    render(<InspectorMetrics target={{ kind: 'Pod', namespace: 'default', name: 'web' }} />)
    expect(screen.getByText(/No metrics for this pod yet/)).toBeTruthy()
  })

  it('renders current pod values', () => {
    useStore.setState({ details: { k: podDetail(1) } } as never)
    render(<InspectorMetrics target={{ kind: 'Pod', namespace: 'default', name: 'web' }} />)
    expect(screen.getByTestId('inspector-metrics').textContent).toContain('12.5%')
  })
})

describe('AppearanceSettings (T117)', () => {
  it('changes theme and persists via SaveUiConfig', () => {
    vi.useFakeTimers()
    render(<AppearanceSettings />)
    fireEvent.change(screen.getByLabelText('Accent'), { target: { value: 'magenta' } })
    fireEvent.click(screen.getByLabelText('Glitch effect'))
    fireEvent.change(screen.getByLabelText('Density'), { target: { value: 'compact' } })
    expect(useStore.getState().ui.theme.accent).toBe('magenta')
    act(() => {
      vi.advanceTimersByTime(SAVE_DEBOUNCE_MS + 1)
    })
    const save = sent.find((c) => c.type === 'SaveUiConfig')
    expect(save).toBeTruthy()
    const ui = (save as Extract<BackendCmd, { type: 'SaveUiConfig' }>).data.ui
    expect(ui.theme.glitch).toBe(true)
    expect(ui.density.override).toBe('compact')
  })

  it('lists shortcuts and About', () => {
    render(<AppearanceSettings />)
    // Every binding in the registry, not a sample: this panel and the `?` overlay used to keep
    // their own lists, which drifted to two entries here against twelve there.
    const text = screen.getByTestId('shortcuts').textContent
    for (const s of SHORTCUTS) {
      expect(text, `settings panel is missing the shortcut ${s.keys}`).toContain(s.keys)
      expect(text, `settings panel is missing the description of ${s.keys}`).toContain(
        s.description,
      )
    }
    expect(screen.getByTestId('about')).toBeTruthy()
    expect(screen.getByTestId('about-version').textContent).toBe('v9.8.7-test')
  })
})

describe('MetricsChart (T103)', () => {
  const pts = [
    { t: 0, a: 1 },
    { t: 1, a: 2 },
  ]
  it('disables animation under reduced motion', () => {
    vi.stubGlobal('matchMedia', (q: string) => ({
      matches: q.includes('reduce'),
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
    }))
    render(<MetricsChart series={[{ key: 'a', label: 'A' }]} points={pts} label="chart" />)
    expect(screen.getByTestId('metrics-chart').getAttribute('data-animated')).toBe('false')
    vi.unstubAllGlobals()
  })
  it('shows a placeholder with fewer than two points', () => {
    render(<MetricsChart series={[{ key: 'a', label: 'A' }]} points={pts.slice(0, 1)} label="c" />)
    expect(screen.getByText(/Collecting samples/)).toBeTruthy()
  })
})
