import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { KedaPanel } from '@/components/panels/KedaPanel'
import type { KedaResourceSummary } from '@/protocol/types'
import { initialData, useStore } from '@/store'

const refreshKeda = vi.fn()

const row = (over: Partial<KedaResourceSummary> = {}): KedaResourceSummary => ({
  kind: 'ScaledObject',
  name: 'queue-worker',
  namespace: 'default',
  target_kind: 'Deployment',
  target_name: 'queue-worker',
  target_found: true,
  current_replicas: 4,
  min_replicas: 1,
  max_replicas: 10,
  paused: false,
  ready: true,
  trigger_types: ['prometheus'],
  age: '3d',
  ...over,
})

beforeEach(() => {
  refreshKeda.mockClear()
  useStore.setState({ ...initialData(), refreshKeda })
})
afterEach(cleanup)

describe('KedaPanel', () => {
  it('waits for a scan and requests a refresh', () => {
    render(<KedaPanel />)
    expect(screen.getByText(/Waiting for the KEDA scan/)).toBeTruthy()
    expect(refreshKeda).toHaveBeenCalled()
  })

  it('shows not-installed distinctly from no-autoscalers', () => {
    useStore.setState({ keda: { overview: { installed: false, scaled_objects: [], scaled_jobs: [] } } })
    const { unmount } = render(<KedaPanel />)
    expect(screen.getByText(/KEDA not detected/)).toBeTruthy()
    unmount()
    useStore.setState({ keda: { overview: { installed: true, scaled_objects: [], scaled_jobs: [] } } })
    render(<KedaPanel />)
    expect(screen.getByTestId('keda-empty')).toBeTruthy()
  })

  it('filters rows by the active namespace', () => {
    useStore.setState({
      keda: {
        overview: {
          installed: true,
          scaled_objects: [row(), row({ name: 'other', namespace: 'prod' })],
          scaled_jobs: [],
        },
      },
    })
    render(<KedaPanel />)
    expect(screen.getAllByTestId('keda-row')).toHaveLength(2)
    cleanup()
    useStore.setState({ session: { ...useStore.getState().session, activeNamespace: 'prod' } })
    render(<KedaPanel />)
    expect(screen.getAllByTestId('keda-row')).toHaveLength(1)
  })
})
