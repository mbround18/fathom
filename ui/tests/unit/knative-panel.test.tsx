import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { KnativePanel } from '@/components/panels/KnativePanel'
import { initialData, useStore } from '@/store'

import { overview, revision } from './knative-fixtures'

const refreshKnative = vi.fn()

beforeEach(() => {
  refreshKnative.mockClear()
  useStore.setState({ ...initialData(), refreshKnative })
})
afterEach(cleanup)

describe('KnativePanel', () => {
  it('shows a waiting state without an overview and requests a refresh', () => {
    render(<KnativePanel />)
    expect(screen.getByText(/Waiting for the Knative scan/)).toBeTruthy()
    expect(refreshKnative).toHaveBeenCalled()
  })

  it('shows the not-installed state and no lists or graph', () => {
    useStore.setState({ knative: { overview: overview({ installed: false }) } })
    render(<KnativePanel />)
    expect(screen.getByText(/Knative Serving CRDs were not found/)).toBeTruthy()
    expect(screen.queryByTestId('knative-services')).toBeNull()
    expect(screen.queryByTestId('knative-revisions')).toBeNull()
    expect(screen.queryByTestId('knative-routes')).toBeNull()
    expect(screen.queryByTestId('knative-traffic-graph')).toBeNull()
  })

  it('renders services, revisions and routes', () => {
    useStore.setState({ knative: { overview: overview() } })
    render(<KnativePanel />)
    expect(screen.getAllByTestId('knative-service-row')).toHaveLength(1)
    expect(screen.getAllByTestId('knative-revision-row')).toHaveLength(2)
    expect(screen.getAllByTestId('knative-route-row')).toHaveLength(1)
    expect(screen.getAllByText('default/hello').length).toBeGreaterThan(0)
    const targets = screen.getAllByTestId('knative-traffic-target').map((e) => e.textContent)
    expect(targets).toEqual(['hello-00001 80%', 'hello-00002 20%'])
  })

  it('explains scaled-to-zero with and without idle time', () => {
    useStore.setState({
      knative: {
        overview: overview({
          revisions: [
            revision({
              actual_replicas: 0,
              desired_replicas: 0,
              scale_reason: { kind: 'ScaledToZero', data: { idle_since: '2026-09-23T00:00:00Z' } },
            }),
            revision({
              name: 'b',
              actual_replicas: 0,
              desired_replicas: 0,
              scale_reason: { kind: 'ScaledToZero', data: { idle_since: null } },
            }),
          ],
        }),
      },
    })
    render(<KnativePanel />)
    const t = screen.getAllByTestId('knative-scale-reason').map((e) => e.textContent)
    expect(t[0]).toContain('idle since 2026-09-23T00:00:00Z')
    expect(t[1]).toBe('Scaled to zero: no traffic')
  })

  it('explains scaling up, at-desired and unavailable honestly', () => {
    useStore.setState({
      knative: {
        overview: overview({
          revisions: [
            revision({
              actual_replicas: 0,
              desired_replicas: 2,
              scale_reason: { kind: 'ScalingUp', data: { pending_requests: 5 } },
            }),
            revision({ name: 'b' }),
            revision({
              name: 'c',
              actual_replicas: 1,
              desired_replicas: 3,
              scale_reason: { kind: 'Unavailable' },
            }),
          ],
        }),
      },
    })
    render(<KnativePanel />)
    const t = screen.getAllByTestId('knative-scale-reason').map((e) => e.textContent)
    expect(t[0]).toBe('Scaling up (5 pending requests)')
    expect(t[1]).toBe('At desired scale')
    expect(t[2]).toBe('Reason unavailable (actual 1 / desired 3)')
  })
})
