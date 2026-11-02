/** CopyBlock: a labelled value with Copy; masked values hide behind dots until Reveal. */
import { expect, test, type Page } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'copy-block'

// The 360 px checks tap, as a phone would.
test.use({ hasTouch: true })

test.beforeEach(async ({ context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
})

const clipboard = (page: Page) => page.evaluate(() => navigator.clipboard.readText())

test('copy: the value is labelled, and Copy from the keyboard copies it and says so', async ({ page }) => {
  await openPiece(page, { piece, state: 'copy' })
  const value = page.locator('[data-copy="address"]')
  await expect(value).toHaveText('widgets.example.test:7443')
  await expect(value).toHaveAccessibleName('Widget address')
  await expect(page.locator('[data-copy-block="address"]')).toBeVisible()
  await expect(value).toHaveAttribute('data-masked', 'false')

  await page.keyboard.press('Tab')
  const copy = page.getByRole('button', { name: 'Copy widget address' })
  await expect(copy).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(copy).toHaveText('Copied')
  await expect(page.getByRole('status').filter({ hasText: 'Widget address copied' })).toHaveCount(1)
  expect(await clipboard(page)).toBe('widgets.example.test:7443')
  // "Copied" is a moment, not a state.
  await expect(copy).toHaveText('Copy', { timeout: 5000 })
})

test('copy: a long value wraps instead of scrolling the page at 360 px, and a tap copies it', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'copy' })
  const [scroll, client] = await page.evaluate(() => [document.documentElement.scrollWidth, document.documentElement.clientWidth])
  expect(scroll).toBeLessThanOrEqual(client)
  await page.getByRole('button', { name: 'Copy install command' }).tap()
  expect(await clipboard(page)).toContain('--name blue-widget')
})

test('masked: dots until Reveal, and Copy always copies the real value', async ({ page }) => {
  await openPiece(page, { piece, state: 'masked' })
  const value = page.locator('[data-copy="token"]')
  await expect(value).toHaveAttribute('data-masked', 'true')
  await expect(value).not.toContainText('wjt_')

  await page.keyboard.press('Tab')
  const reveal = page.getByRole('button', { name: 'Reveal join token' })
  await expect(reveal).toBeFocused()
  await expect(reveal).toHaveAttribute('aria-pressed', 'false')

  await page.keyboard.press('Tab')
  await page.keyboard.press('Space')
  expect(await clipboard(page)).toBe('wjt_7f3a9c2e81b44d0f9a6e5c1d2b3a4f5e')
  await expect(value).toHaveAttribute('data-masked', 'true')

  await page.keyboard.press('Shift+Tab')
  await page.keyboard.press('Enter')
  const hide = page.getByRole('button', { name: 'Hide join token' })
  await expect(hide).toHaveAttribute('aria-pressed', 'true')
  await expect(value).toHaveText('wjt_7f3a9c2e81b44d0f9a6e5c1d2b3a4f5e')
  await expect(value).toHaveAttribute('data-masked', 'false')
  await page.keyboard.press('Enter')
  await expect(value).toHaveAttribute('data-masked', 'true')
})

for (const state of ['copy', 'masked']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}

test('masked is accessible revealed', async ({ page }) => {
  await expectAccessible(
    page,
    { piece, state: 'masked' },
    {
      prepare: async (p) => {
        await p.getByRole('button', { name: 'Reveal join token' }).click()
      },
    },
  )
})
