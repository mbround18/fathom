/** 034-simultaneous-federation US2: fleet cards show a live/polled indicator; un-federating
 * falls back to polled without duplicate cards. */
import { expect, test } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, open } from './helpers'

const base = {
  error: null,
  flux: null,
  restricted: [],
  cluster_version: 'v1.31',
  node_ready_count: 3,
  node_total_count: 3,
  unhealthy_pod_count: 0,
  health: 'Healthy' as const,
}

test('live card and polled card, fallback on un-federate without duplicates', async ({ page }) => {
  await open(page, 'manual')
  await emit(
    page,
    batch([
      system(1, { type: 'ContextsUpdated', data: { contexts: ['prod', 'dev'], current: 'prod' } }),
    ]),
  )
  await page.getByTestId('fleet-open').click()
  await emit(
    page,
    batch([
      system(2, {
        type: 'FleetSummaryUpdated',
        data: { ...base, context: 'prod', last_updated: '2026-09-24T00:00:00Z', live: true, plugin_error: null },
      }),
      system(3, {
        type: 'FleetSummaryUpdated',
        data: { ...base, context: 'dev', last_updated: '2026-09-24T00:00:00Z', live: false, plugin_error: null },
      }),
    ]),
  )
  const card = (n: string) =>
    page.locator(`[data-testid="fleet-context-card"][data-context-name="${n}"]`)
  await expect(page.getByTestId('fleet-context-card')).toHaveCount(2)
  await expect(card('prod').getByTestId('fleet-tier')).toHaveAttribute('data-tier', 'live')
  await expect(card('dev').getByTestId('fleet-tier')).toHaveAttribute('data-tier', 'polled')

  // prod un-federated: its poller resumes and reports a polled summary.
  await emit(
    page,
    batch([
      system(4, {
        type: 'FleetSummaryUpdated',
        data: { ...base, context: 'prod', last_updated: '2026-09-24T00:00:30Z', live: false, plugin_error: null },
      }),
    ]),
  )
  await expect(card('prod').getByTestId('fleet-tier')).toHaveAttribute('data-tier', 'polled')
  await expect(page.getByTestId('fleet-context-card')).toHaveCount(2)
})
