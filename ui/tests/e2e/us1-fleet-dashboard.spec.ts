/** 027-fleet-rollup-dashboard US1: two contexts render simultaneously, active context unchanged. */
import { expect, test } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, open, sentTypes } from './helpers'

test('fleet dashboard shows both contexts without switching the active one', async ({ page }) => {
  await open(page, 'manual')
  await emit(
    page,
    batch([
      system(1, { type: 'ContextsUpdated', data: { contexts: ['prod', 'dev'], current: 'prod' } }),
    ]),
  )
  await page.getByTestId('fleet-open').click()
  await expect(page.getByTestId('fleet-context-card')).toHaveCount(2)
  await expect
    .poll(async () => (await sentTypes(page)).includes('StartFleetPolling'))
    .toBe(true)

  const base = {
    error: null,
    flux: null,
    last_updated: '2026-09-23T00:00:00Z',
    cluster_version: 'v1.31',
  }
  await emit(
    page,
    batch([
      system(2, {
        type: 'FleetSummaryUpdated',
        data: {
          ...base,
          context: 'prod',
          health: 'Healthy',
          node_ready_count: 3,
          node_total_count: 3,
          unhealthy_pod_count: 0,
        },
      }),
      system(3, {
        type: 'FleetSummaryUpdated',
        data: {
          ...base,
          context: 'dev',
          health: 'Degraded',
          node_ready_count: 2,
          node_total_count: 3,
          unhealthy_pod_count: 4,
        },
      }),
    ]),
  )
  const card = (n: string) =>
    page.locator(`[data-testid="fleet-context-card"][data-context-name="${n}"]`)
  await expect(card('prod')).toHaveAttribute('data-health', 'Healthy')
  await expect(card('dev')).toHaveAttribute('data-health', 'Degraded')
  await expect(card('dev').getByTestId('fleet-unhealthy-pods')).toHaveText('4')
  await expect(card('prod')).toHaveAttribute('aria-pressed', 'true')
  await expect(card('dev')).toHaveAttribute('aria-pressed', 'false')
  expect(await sentTypes(page)).not.toContain('SetContext')
})
