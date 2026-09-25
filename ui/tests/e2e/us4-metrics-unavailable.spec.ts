/**
 * US4 (T097): with no metrics source reporting, the Metrics pane shows `MetricsUnavailable`, not
 * empty charts or a blank table. (The cluster Overview no longer embeds metric charts: it shows
 * counts and warnings only, so only the Metrics pane carries this state.)
 */
import { expect, test } from '@playwright/test'

import { expectNoHorizontalScroll, navigateTo, open } from './helpers'

test.describe('US4 metrics unavailable', () => {
  test('Metrics pane shows the unavailable state instead of an empty table', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Metrics', 'Integrations')
    const pane = page.locator('[data-slot="pane"][data-section="integrations/metrics"]').first()
    const state = pane.locator('[data-pane-state="metrics-unavailable"]')
    await expect(state).toBeVisible()
    await expect(state).toContainText('Metrics unavailable')
    await expect(state).toContainText('No metrics received yet')
    await expect(pane.getByTestId('metrics-status')).toContainText('not tested')
    await expect(pane.locator('table')).toHaveCount(0)
    await expectNoHorizontalScroll(page)
  })
})
