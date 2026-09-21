import { expect, test } from '@playwright/test'

import type { ShellExpectation } from '../../playwright.config'

test.describe('app shell', () => {
  test('renders the width class, slots, and chrome without horizontal overflow', async ({
    page,
  }, info) => {
    const want = info.project.metadata as ShellExpectation
    await page.goto('/')

    await expect(page.locator('html')).toHaveAttribute('data-width-class', want.widthClass)
    await expect(page.getByRole('img', { name: 'Fathom' })).toBeVisible()
    await expect(page.getByTestId('status-bar')).toBeVisible()
    await expect(page.locator('[data-slot="pane"][data-slot-index]')).toHaveCount(want.slots)
    await expect(page.getByTestId('dock-tabs')).toHaveCount(want.dock ? 1 : 0)

    const { scrollWidth, clientWidth } = await page.evaluate(() => ({
      scrollWidth: document.documentElement.scrollWidth,
      clientWidth: document.documentElement.clientWidth,
    }))
    expect(scrollWidth).toBeLessThanOrEqual(clientWidth)
  })

  test('pane state survives narrow → wide → narrow (FR-007)', async ({ page }, info) => {
    test.skip(!info.project.name.endsWith('-phone'), 'drives its own viewport sizes')
    await page.goto('/')

    const filter = page.locator('[data-pane-kind="list"]').first().getByRole('searchbox')
    await filter.fill('nginx')

    await page.setViewportSize({ width: 2560, height: 1440 })
    await expect(page.locator('html')).toHaveAttribute('data-width-class', 'ultra')
    await expect(
      page.locator('[data-pane-kind="list"]').first().getByRole('searchbox'),
    ).toHaveValue('nginx')

    await page.setViewportSize({ width: 390, height: 844 })
    await expect(page.locator('html')).toHaveAttribute('data-width-class', 'phone')
    await expect(filter).toHaveValue('nginx')
  })
})
