/**
 * 032 (D): status-bar notices, empty/not-installed states, and the disconnected screen.
 */
import { expect, test } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { chooseScope, emit, open, openToolPane } from './helpers'

const LONG = `Something failed: ${'very long detail '.repeat(20)}`

test.describe('error and empty states', () => {
  test('a status-bar error truncates, offers Details, and clears on namespace change', async ({
    page,
  }) => {
    await open(page)
    await emit(
      page,
      batch([system(40, { type: 'Notification', data: { level: 'Error', message: LONG } })]),
    )
    const notice = page.getByTestId('status-notice')
    await expect(notice).toBeVisible()
    await page.getByTestId('status-notice-details').click()
    await expect(page.getByTestId('status-notice-popover')).toContainText('very long detail')
    await page.keyboard.press('Escape')

    await chooseScope(page, 'Namespace', 'payments')
    await expect(page.getByTestId('status-notice')).toHaveCount(0)
  })

  test('the disconnected screen never shows raw exception text', async ({ page }) => {
    await open(page)
    await page.evaluate(() => window.__fathomFake?.setBackendGone(true))
    await chooseScope(page, 'Context', 'kind-b')
    const state = page.locator('[data-pane-state="disconnected"]').first()
    await expect(state).toBeVisible()
    await expect(state).not.toContainText(/TypeError|Error:|undefined|at \w+\(/)
  })

  test('the logs tool explains what to do when no pod is selected', async ({ page }) => {
    await open(page)
    await openToolPane(page, 'Logs')
    await expect(page.getByText('Select a pod to stream logs.').first()).toBeVisible()
  })
})
