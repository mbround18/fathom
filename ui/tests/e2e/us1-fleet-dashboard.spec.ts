/** 027-fleet-rollup-dashboard US1: two contexts render simultaneously, active context unchanged. */
import { expect, test } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, open, openFleet, sentTypes } from './helpers'

// Radix unmounts a closing dialog only after its exit animation's `animationend`; on a CPU-starved
// runner that event lags by seconds and `toBeHidden()` flaked. Reduced motion (effects.css) makes
// the dialog unmount synchronously, so the close assertion tests the behaviour, not animation timing.
test.use({ reducedMotion: 'reduce' })

test('fleet dashboard shows both contexts without switching the active one', async ({ page }) => {
  await open(page, 'manual')
  await emit(
    page,
    batch([
      system(1, { type: 'ContextsUpdated', data: { contexts: ['prod', 'dev'], current: 'prod' } }),
    ]),
  )
  await openFleet(page)
  await expect(page.getByTestId('fleet-context-card')).toHaveCount(2)
  await expect.poll(async () => (await sentTypes(page)).includes('StartFleetPolling')).toBe(true)

  const base = {
    error: null,
    plugin_error: null,
    flux: null,
    restricted: [],
    last_updated: '2026-09-23T00:00:00Z',
    live: false,
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

test('restricted group shows "no access"; clicking a card switches and closes the dialog', async ({
  page,
}) => {
  await open(page, 'manual')
  await emit(
    page,
    batch([
      system(1, { type: 'ContextsUpdated', data: { contexts: ['prod', 'dev'], current: 'prod' } }),
    ]),
  )
  await openFleet(page)
  await emit(
    page,
    batch([
      system(2, {
        type: 'FleetSummaryUpdated',
        data: {
          context: 'dev',
          health: 'Healthy',
          error: null,
          plugin_error: null,
          flux: null,
          restricted: ['pods'],
          last_updated: new Date().toISOString(),
          live: false,
          cluster_version: 'v1.31',
          node_ready_count: 3,
          node_total_count: 3,
          unhealthy_pod_count: null,
        },
      }),
    ]),
  )
  const dev = page.locator('[data-testid="fleet-context-card"][data-context-name="dev"]')
  await expect(dev.getByTestId('fleet-unhealthy-pods')).toHaveText('no access')
  await expect(dev.getByTestId('fleet-nodes')).toHaveText('3/3')
  await expect(dev.getByTestId('fleet-version')).toHaveText('v1.31')
  await expect(dev.getByTestId('fleet-updated')).toContainText('ago')
  await dev.click()
  await expect(page.getByTestId('fleet-dashboard')).toBeHidden()
  await expect.poll(async () => (await sentTypes(page)).includes('SetContext')).toBe(true)
})
