/** 027-fleet-rollup-dashboard: `FleetSummaryUpdated` handler + fleet polling actions. */
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { FleetContextSummary } from '@/protocol/types'
import { initialData, useStore } from '@/store'
import { resetActions } from '@/store/actions'
import { ingest } from '@/store/ingest'

import { batch, system } from '../fixtures/builders'

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
})

const summary = (
  context: string,
  over: Partial<FleetContextSummary> = {},
): FleetContextSummary => ({
  context,
  health: 'Healthy',
  error: null,
  plugin_error: null,
  node_ready_count: 3,
  node_total_count: 3,
  unhealthy_pod_count: 0,
  cluster_version: 'v1.31',
  flux: null,
  restricted: [],
  last_updated: '2026-09-23T00:00:00Z',
  live: false,
  ...over,
})

describe('FleetSummaryUpdated', () => {
  it('upserts one context and leaves others untouched (last write wins)', () => {
    ingest(batch([system(1, { type: 'FleetSummaryUpdated', data: summary('prod') })]))
    ingest(batch([system(2, { type: 'FleetSummaryUpdated', data: summary('dev') })]))
    ingest(
      batch([
        system(3, {
          type: 'FleetSummaryUpdated',
          data: summary('prod', { health: 'Degraded', unhealthy_pod_count: 2 }),
        }),
      ]),
    )
    const { fleetSummaries } = useStore.getState().session
    expect(fleetSummaries.prod?.health).toBe('Degraded')
    expect(fleetSummaries.prod?.unhealthy_pod_count).toBe(2)
    expect(fleetSummaries.dev?.health).toBe('Healthy')
  })

  it('start/stop send the commands and stop clears summaries', () => {
    const send = vi.fn()
    useStore.setState({ send })
    ingest(batch([system(1, { type: 'FleetSummaryUpdated', data: summary('prod') })]))
    useStore.getState().startFleetPolling()
    useStore.getState().stopFleetPolling()
    expect(send).toHaveBeenCalledWith({ type: 'StartFleetPolling' })
    expect(send).toHaveBeenCalledWith({ type: 'StopFleetPolling' })
    expect(useStore.getState().session.fleetSummaries).toEqual({})
  })
})
