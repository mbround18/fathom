/** 034-simultaneous-federation US3: the restored set is shown after (re)start, an over-cap
 * warning never disconnects, and restricted metrics render as "no access". */
import { expect, test } from '@playwright/test'

import type { FederationContextRollup } from '../../src/protocol/types'
import { batch, system } from '../fixtures/builders'
import { emit, open, sentTypes } from './helpers'

const rollup = (
  context: string,
  over: Partial<FederationContextRollup> = {},
): FederationContextRollup => ({
  context,
  state: 'Live',
  generation: 1,
  error: null,
  nodes_ready: 1,
  nodes_total: 1,
  pods_running: 5,
  pods_pending: 0,
  pods_failed: 0,
  pods_unhealthy: 0,
  deployments_unavailable: 0,
  warning_events_1h: 0,
  cluster_version: 'v1.31',
  restricted: [],
  last_updated: '2026-09-24T00:00:00Z',
  ...over,
})

const combined = {
  contributing_contexts: 1,
  total_contexts: 1,
  nodes_ready: 1,
  nodes_total: 1,
  pods_running: 5,
  pods_pending: 0,
  pods_failed: 0,
  pods_unhealthy: 0,
  deployments_unavailable: 0,
  warning_events_1h: 0,
}

test('restored set, unavailable context, over-cap banner, no-access metric', async ({ page }) => {
  await open(page, 'manual')
  await emit(
    page,
    batch([
      system(1, { type: 'ContextsUpdated', data: { contexts: ['prod'], current: 'prod' } }),
      system(2, {
        type: 'FederationStateUpdated',
        data: { federated: ['gone', 'prod'], soft_cap: 8, over_cap: false },
      }),
      system(3, {
        type: 'FederationRollupUpdated',
        data: {
          context: rollup('prod', { restricted: ['nodes'], nodes_ready: null, nodes_total: null }),
          combined,
        },
      }),
      system(4, {
        type: 'FederationRollupUpdated',
        data: {
          context: rollup('gone', {
            state: 'Unreachable',
            error: 'Context not found in kubeconfig',
            nodes_ready: null,
            nodes_total: null,
            pods_running: null,
            pods_pending: null,
            pods_failed: null,
            pods_unhealthy: null,
            deployments_unavailable: null,
            warning_events_1h: null,
            cluster_version: null,
          }),
          combined,
        },
      }),
    ]),
  )
  await page.getByTestId('fleet-open').click()
  const row = (n: string) =>
    page.locator(`[data-testid="federation-row"][data-context-name="${n}"]`)
  await expect(row('prod').getByTestId('federation-nodes')).toHaveText('no access')
  await expect(row('gone')).toHaveAttribute('data-state', 'Unreachable')
  await expect(row('gone').getByTestId('federation-error')).toHaveText(
    'Context not found in kubeconfig',
  )
  // The unavailable context stays until the operator removes it.
  await row('gone').getByTestId('federation-toggle').click()
  await expect.poll(async () => (await sentTypes(page)).includes('UnfederateContext')).toBe(true)

  await emit(
    page,
    batch([
      system(5, {
        type: 'FederationStateUpdated',
        data: { federated: ['prod'], soft_cap: 1, over_cap: true },
      }),
    ]),
  )
  await expect(page.getByTestId('federation-over-cap')).toBeVisible()
  await expect(row('prod')).toHaveAttribute('data-state', 'Live')
})
