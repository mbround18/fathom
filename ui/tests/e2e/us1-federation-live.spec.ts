/** 034-simultaneous-federation US1: two contexts live at once; one errors while the other stays
 * live; the primary context is never switched. */
import { expect, test } from '@playwright/test'

import type { FederationContextRollup } from '../../src/protocol/types'
import { batch, system } from '../fixtures/builders'
import { emit, open, openFleet, sentTypes } from './helpers'

const rollup = (
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

const combined = (n: number) => ({
  contributing_contexts: n,
  total_contexts: 2,
  nodes_ready: 3 * n,
  nodes_total: 3 * n,
  pods_running: 20 * n,
  pods_pending: n,
  pods_failed: 0,
  pods_unhealthy: 2 * n,
  deployments_unavailable: 0,
  warning_events_1h: 4 * n,
})

test('two federated contexts stay live independently; primary unchanged', async ({ page }) => {
  await open(page, 'manual')
  await emit(
    page,
    batch([
      system(1, { type: 'ContextsUpdated', data: { contexts: ['prod', 'dev'], current: 'prod' } }),
    ]),
  )
  await openFleet(page)
  await expect
    .poll(async () => (await sentTypes(page)).includes('RequestFederationSnapshot'))
    .toBe(true)

  await page
    .locator('[data-testid="federation-off-row"][data-context-name="dev"]')
    .getByTestId('federation-toggle')
    .click()
  await expect.poll(async () => (await sentTypes(page)).includes('FederateContext')).toBe(true)

  await emit(
    page,
    batch([
      system(2, {
        type: 'FederationStateUpdated',
        data: { federated: ['dev', 'prod'], soft_cap: 8, over_cap: false },
      }),
      system(3, {
        type: 'FederationRollupUpdated',
        data: { context: rollup('prod'), combined: combined(1) },
      }),
      system(4, {
        type: 'FederationRollupUpdated',
        data: { context: rollup('dev'), combined: combined(2) },
      }),
    ]),
  )
  const row = (n: string) =>
    page.locator(`[data-testid="federation-row"][data-context-name="${n}"]`)
  await expect(row('prod')).toHaveAttribute('data-state', 'Live')
  await expect(row('dev')).toHaveAttribute('data-state', 'Live')
  await expect(page.getByTestId('federation-combined-contexts')).toHaveText('2/2')

  // dev fails; prod is untouched.
  await emit(
    page,
    batch([
      system(5, {
        type: 'FederationRollupUpdated',
        data: {
          context: rollup('dev', {
            generation: 1,
            state: 'Unreachable',
            error: 'dial tcp: connection refused',
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
          combined: combined(1),
        },
      }),
    ]),
  )
  await expect(row('dev')).toHaveAttribute('data-state', 'Unreachable')
  await expect(row('dev').getByTestId('federation-error')).toHaveText(
    'dial tcp: connection refused',
  )
  await expect(row('dev').getByTestId('federation-nodes')).toHaveCount(0)
  await expect(row('prod')).toHaveAttribute('data-state', 'Live')
  await expect(row('prod').getByTestId('federation-pods-unhealthy')).toHaveText('2')
  await expect(page.getByTestId('federation-combined-contexts')).toHaveText('1/2')
  expect(await sentTypes(page)).not.toContain('SetContext')
})
