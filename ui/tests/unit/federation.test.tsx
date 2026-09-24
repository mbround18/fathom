/** 034-simultaneous-federation: store slice, stale-event dropping, and row/card rendering. */
import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { FederationRow } from '@/components/fleet/FederationRow'
import { FleetContextCard } from '@/components/fleet/FleetContextCard'
import type {
  FederationCombinedRollup,
  FederationContextRollup,
  FleetContextSummary,
} from '@/protocol/types'
import { initialData, useStore } from '@/store'
import { resetActions } from '@/store/actions'
import { ingest } from '@/store/ingest'

import { batch, system } from '../fixtures/builders'

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
})

export const rollup = (
  context: string,
  over: Partial<FederationContextRollup> = {},
): FederationContextRollup => ({
  context,
  state: 'Live',
  generation: 1,
  error: null,
  nodes_ready: 3,
  nodes_total: 3,
  pods_running: 20,
  pods_pending: 1,
  pods_failed: 0,
  pods_unhealthy: 2,
  deployments_unavailable: 0,
  warning_events_1h: 4,
  cluster_version: 'v1.31',
  restricted: [],
  last_updated: '2026-09-24T00:00:00Z',
  ...over,
})

const combined = (over: Partial<FederationCombinedRollup> = {}): FederationCombinedRollup => ({
  contributing_contexts: 1,
  total_contexts: 1,
  nodes_ready: 3,
  nodes_total: 3,
  pods_running: 20,
  pods_pending: 1,
  pods_failed: 0,
  pods_unhealthy: 2,
  deployments_unavailable: 0,
  warning_events_1h: 4,
  ...over,
})

const state = (federated: string[], over_cap = false) =>
  system(1, { type: 'FederationStateUpdated', data: { federated, soft_cap: 8, over_cap } })

describe('federation store slice', () => {
  it('adopts the federated set and rollups, and drops rollups of removed contexts', () => {
    ingest(batch([state(['a', 'b'])]))
    ingest(
      batch([
        system(2, {
          type: 'FederationRollupUpdated',
          data: { context: rollup('a'), combined: combined() },
        }),
        system(3, {
          type: 'FederationRollupUpdated',
          data: { context: rollup('b'), combined: combined({ contributing_contexts: 2 }) },
        }),
      ]),
    )
    const fed = () => useStore.getState().session.federation
    expect(Object.keys(fed().rollups).sort()).toEqual(['a', 'b'])
    expect(fed().combined?.contributing_contexts).toBe(2)
    ingest(batch([state(['a'])]))
    expect(Object.keys(fed().rollups)).toEqual(['a'])
    ingest(batch([state([])]))
    expect(fed().combined).toBeNull()
  })

  it('drops events older than the newest generation for a context', () => {
    ingest(batch([state(['a'])]))
    const send = (seq: number, generation: number, pods: number) =>
      ingest(
        batch([
          system(seq, {
            type: 'FederationRollupUpdated',
            data: {
              context: rollup('a', { generation, pods_running: pods }),
              combined: combined(),
            },
          }),
        ]),
      )
    send(2, 2, 10)
    send(3, 1, 99)
    expect(useStore.getState().session.federation.rollups.a?.pods_running).toBe(10)
    send(4, 2, 11)
    expect(useStore.getState().session.federation.rollups.a?.pods_running).toBe(11)
  })

  it('surfaces over_cap without changing the set', () => {
    ingest(batch([state(['a', 'b'], true)]))
    expect(useStore.getState().session.federation.overCap).toBe(true)
    expect(useStore.getState().session.federation.federated).toEqual(['a', 'b'])
  })

  it('sends federate/unfederate/snapshot commands', () => {
    const send = vi.fn()
    useStore.setState({ send })
    useStore.getState().federateContext('a')
    useStore.getState().unfederateContext('a')
    useStore.getState().requestFederationSnapshot()
    expect(send).toHaveBeenCalledWith({ type: 'FederateContext', data: { context: 'a' } })
    expect(send).toHaveBeenCalledWith({ type: 'UnfederateContext', data: { context: 'a' } })
    expect(send).toHaveBeenCalledWith({ type: 'RequestFederationSnapshot' })
  })
})

describe('FederationRow', () => {
  it('shows numbers only when live; failures show the verbatim error and no numbers', () => {
    const { rerender } = render(<FederationRow rollup={rollup('a')} toggle={null} />)
    expect(screen.getByTestId('federation-nodes')).toHaveTextContent('3/3')
    rerender(
      <FederationRow
        rollup={rollup('a', {
          state: 'Unreachable',
          error: 'dial tcp 10.0.0.1:6443: i/o timeout',
          nodes_ready: null,
          nodes_total: null,
        })}
        toggle={null}
      />,
    )
    expect(screen.getByTestId('federation-error')).toHaveTextContent(
      'dial tcp 10.0.0.1:6443: i/o timeout',
    )
    expect(screen.queryByTestId('federation-nodes')).toBeNull()
  })

  it('marks last-known values stale while reconnecting', () => {
    render(<FederationRow rollup={rollup('a', { state: 'Reconnecting' })} toggle={null} />)
    expect(screen.getByTestId('federation-stale')).toBeInTheDocument()
  })

  it('renders "no access" for restricted metrics', () => {
    render(
      <FederationRow
        rollup={rollup('a', {
          restricted: ['nodes', 'events'],
          nodes_ready: null,
          nodes_total: null,
          warning_events_1h: null,
        })}
        toggle={null}
      />,
    )
    expect(screen.getByTestId('federation-nodes')).toHaveTextContent('no access')
    expect(screen.getByTestId('federation-warnings')).toHaveTextContent('no access')
  })
})

describe('FleetContextCard tier indicator', () => {
  const summary = (live: boolean): FleetContextSummary => ({
    context: 'a',
    health: 'Healthy',
    error: null,
    node_ready_count: 1,
    node_total_count: 1,
    unhealthy_pod_count: 0,
    cluster_version: 'v1',
    flux: null,
    restricted: [],
    last_updated: '',
    live,
  })
  it.each([
    [true, 'live', 'Live'],
    [false, 'polled', 'Polled'],
  ])('live=%s renders %s', (live, tier, label) => {
    render(<FleetContextCard summary={summary(live)} active={false} onSelect={() => {}} now={0} />)
    const el = screen.getByTestId('fleet-tier')
    expect(el).toHaveAttribute('data-tier', tier)
    expect(el).toHaveTextContent(label)
  })
})
