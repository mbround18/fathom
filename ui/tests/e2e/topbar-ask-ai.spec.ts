/**
 * 032 (A): the top-bar right cluster (Search, Ask AI, Fleet, overflow) and the Ask AI sheet.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, open } from './helpers'

const onPhone = async (page: Page) =>
  (await page.locator('html').getAttribute('data-width-class')) === 'phone'

test.describe('top bar and Ask AI', () => {
  test('Search button opens the command palette', async ({ page }) => {
    await open(page)
    await page.getByTestId('palette-open').click()
    await expect(page.getByRole('dialog')).toBeVisible()
    await expect(page.getByRole('combobox').last()).toBeFocused()
  })

  test('unconfigured assistant shows a setup popover that leads to auth setup', async ({
    page,
  }) => {
    await open(page)
    if (await onPhone(page)) {
      await page.getByTestId('overflow-menu').click()
      await page.getByRole('menuitem', { name: /Ask AI/ }).click()
    } else {
      await page.getByRole('button', { name: 'Ask AI (not set up)' }).click()
      await page.getByTestId('ask-ai-setup').click()
    }
    await expect(page.getByTestId('ask-ai-sheet')).toBeVisible()
    await expect(page.getByTestId('agent-ops-auth-setup')).toBeVisible()
  })

  test('configured assistant opens a sheet with a context chip', async ({ page }) => {
    await open(page)
    await emit(
      page,
      batch([
        system(20, {
          type: 'AgentAuthModeUpdated',
          data: { mode: { type: 'Anthropic', data: { key_configured: true } } },
        }),
      ]),
    )
    if (await onPhone(page)) {
      await page.getByTestId('overflow-menu').click()
      await page.getByRole('menuitem', { name: /Ask AI/ }).click()
    } else {
      await page.getByTestId('ask-ai-open').click()
    }
    const sheet = page.getByTestId('ask-ai-sheet')
    await expect(sheet).toBeVisible()
    await expect(sheet.getByTestId('ask-ai-context-chip')).toContainText('kind-a')
    const vw = page.viewportSize()?.width ?? 0
    await expect
      .poll(async () => {
        const box = await sheet.boundingBox()
        return (box?.x ?? 0) + (box?.width ?? 0) <= vw + 1
      })
      .toBe(true)
  })

  test('palette is at least 560px wide on desktop', async ({ page }) => {
    await open(page)
    test.skip((page.viewportSize()?.width ?? 0) < 1024, 'desktop only')
    await page.keyboard.press('Control+k')
    await expect
      .poll(async () => (await page.getByRole('dialog').boundingBox())?.width ?? 0)
      .toBeGreaterThanOrEqual(560)
  })

  test('palette lists tool entries', async ({ page }) => {
    await open(page)
    await page.getByTestId('palette-open').click()
    const input = page.getByRole('combobox').last()
    await input.fill('Knative')
    await expect(page.getByRole('option', { name: /Knative/ }).first()).toBeVisible()
    // Feedback #12: Velero stays out of the palette until it is detected.
    await input.fill('Velero')
    await expect(page.getByRole('option', { name: /Velero/ })).toHaveCount(0)
  })
})
