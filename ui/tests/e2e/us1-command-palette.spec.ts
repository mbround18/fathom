/**
 * 016-command-palette: Cmd/Ctrl+K jumps to a resource, a namespace, or a section.
 */
import { expect, test, type Page } from '@playwright/test'

import { PODS_500 } from '../fixtures'
import { expectRowCount, expectScope, inspector, open } from './helpers'

const POD = PODS_500[0]
if (!POD) throw new Error('fixture')

const dialog = (page: Page) => page.getByRole('dialog')
const input = (page: Page) => page.locator('[cmdk-input]')

test.describe('Command palette', () => {
  test('Ctrl+K opens the palette; selecting a pod opens its inspector', async ({ page }) => {
    await open(page)
    await expectRowCount(page, 500)
    await page.keyboard.press('Control+k')
    await expect(dialog(page)).toBeVisible()
    await input(page).fill(POD.name)
    const item = page.locator('[cmdk-item]').first()
    await expect(item).toContainText(POD.name)
    await page.keyboard.press('Enter')
    await expect(dialog(page)).toBeHidden()
    await expect(inspector(page)).toContainText(POD.name)
  })

  test('selecting a namespace sets the namespace filter', async ({ page }) => {
    await open(page)
    await expectRowCount(page, 500)
    await page.keyboard.press('Control+k')
    await input(page).fill('payments')
    await page.locator('[cmdk-item]', { hasText: 'payments' }).first().click()
    await expectScope(page, 'Namespace', 'payments')
  })

  test('selecting a section navigates to it', async ({ page }) => {
    await open(page)
    await page.keyboard.press('Control+k')
    await input(page).fill('deployments')
    await page.locator('[cmdk-item]', { hasText: 'Deployments' }).first().click()
    await expect(
      page.locator('[data-testid="resource-table"][data-section="workloads/deployments"]').first(),
    ).toBeVisible()
  })

  test('shows one empty state when nothing matches', async ({ page }) => {
    await open(page)
    await page.keyboard.press('Control+k')
    await input(page).fill('zzzzqqqq-nomatch')
    await expect(page.locator('[cmdk-empty]')).toHaveCount(1)
  })
})
