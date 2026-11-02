/**
 * Spec 065 T030 (FR-013): Fathom's panes on the shared kit. A list that fails to load shows the
 * kit's failed state in its own pane, and a pane that throws is caught by its own boundary; in
 * both cases the navigator and the other sections keep working.
 */
import { expect, test } from '@playwright/test'

import { PODS_500, PODS_FORBIDDEN } from '../fixtures'
import {
  DEPLOYMENTS,
  expectRowCount,
  listPane,
  navigateTo,
  open,
  table,
} from './helpers'

test.describe('065 kit panes', () => {
  test('a list that fails to load shows the kit failed state; other sections still open', async ({
    page,
  }) => {
    await open(page, 'pods-error')
    const failed = listPane(page).locator('[data-pane-state="error"]')
    await expect(failed).toBeVisible()
    await expect(failed).toContainText('Failed to load pods.')
    await expect(failed).toContainText(PODS_FORBIDDEN)
    await expect(failed.getByRole('button', { name: 'Retry' })).toBeVisible()

    await navigateTo(page, 'Deployments')
    await expect(table(page, DEPLOYMENTS)).toBeVisible()
    await expect(listPane(page, DEPLOYMENTS).locator('[data-pane-state="error"]')).toHaveCount(0)
  })

  test('a pane that throws is contained to that pane', async ({ page }) => {
    const errors: string[] = []
    page.on('pageerror', (e) => errors.push(e.message))
    // Fake builds only: `?crash=<section>` makes that section's list pane throw while rendering.
    await page.goto(`/?scenario=default&crash=${DEPLOYMENTS}`)
    await page.waitForFunction(() => window.__fathomFake !== undefined)
    await expectRowCount(page, PODS_500.length)

    await navigateTo(page, 'Deployments')
    const crashed = listPane(page, DEPLOYMENTS).locator('[data-pane-state="error"]')
    await expect(crashed).toContainText('Something went wrong showing this page.')
    await expect(crashed).toContainText(`Fake crash in ${DEPLOYMENTS}`)
    // The pane keeps its header, and the shell (with the navigator) is still up.
    await expect(listPane(page, DEPLOYMENTS).locator('[data-pane-header]')).toBeVisible()
    await expect(page.locator('[data-slot="pane"]').first()).toBeVisible()

    // Showing something else in the pane clears the boundary.
    await navigateTo(page, 'Pods')
    await expectRowCount(page, PODS_500.length)
    await expect(listPane(page).locator('[data-pane-state="error"]')).toHaveCount(0)
    expect(errors.filter((m) => !m.includes('Fake crash'))).toEqual([])
  })
})
